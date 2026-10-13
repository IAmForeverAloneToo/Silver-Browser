//! The URL parser against web-platform-tests' URL data in `data/`
//! (see `data/README.md`): every case is run, and the count of cases is
//! checked so that none is skipped without saying so.

#![forbid(unsafe_code)]

mod support;

use net::url::{Host, Url};
use serde_json::Value;

use support::objects as load;

/// What the URL API's getters would return, from the parsed URL.
struct Getters {
    href: String,
    protocol: String,
    username: String,
    password: String,
    host: String,
    hostname: String,
    port: String,
    pathname: String,
    search: String,
    hash: String,
    origin: String,
}

impl Getters {
    fn of(url: &Url) -> Getters {
        let hostname = url.host().map(Host::to_string).unwrap_or_default();
        let port = url.port().map(|p| p.to_string()).unwrap_or_default();
        let host = if port.is_empty() {
            hostname.clone()
        } else {
            format!("{hostname}:{port}")
        };
        let prefixed = |prefix: &str, value: Option<&str>| match value {
            Some(value) if !value.is_empty() => format!("{prefix}{value}"),
            _ => String::new(),
        };
        Getters {
            href: url.as_str().to_owned(),
            protocol: format!("{}:", url.scheme()),
            username: url.username().to_owned(),
            password: url.password().to_owned(),
            host,
            hostname,
            port,
            pathname: url.path_str(),
            search: prefixed("?", url.query()),
            hash: prefixed("#", url.fragment()),
            origin: url.origin().serialization(),
        }
    }

    fn get(&self, field: &str) -> &str {
        match field {
            "href" => &self.href,
            "protocol" => &self.protocol,
            "username" => &self.username,
            "password" => &self.password,
            "host" => &self.host,
            "hostname" => &self.hostname,
            "port" => &self.port,
            "pathname" => &self.pathname,
            "search" => &self.search,
            "hash" => &self.hash,
            "origin" => &self.origin,
            _ => panic!("no getter {field}"),
        }
    }
}

const FIELDS: [&str; 11] = [
    "href", "origin", "protocol", "username", "password", "host", "hostname", "port", "pathname",
    "search", "hash",
];

#[test]
fn urltestdata() {
    let cases = load("urltestdata.json");
    assert_eq!(cases.len(), 896, "every case is run");
    let mut failures = Vec::new();
    for case in &cases {
        let input = case["input"].as_str().expect("an input");
        let parsed = match case["base"].as_str() {
            Some(base) => Url::parse(base).and_then(|base| base.join(input)),
            None => Url::parse(input),
        };
        let expect_failure = case["failure"].as_bool().unwrap_or(false);
        match (parsed, expect_failure) {
            (Err(_), true) => {}
            (Ok(url), true) => {
                failures.push(format!("{input:?} parsed as {url}, expected failure"))
            }
            (Err(e), false) => failures.push(format!(
                "{input:?} failed ({e:?}), expected {}",
                case["href"]
            )),
            (Ok(url), false) => {
                let got = Getters::of(&url);
                for field in FIELDS {
                    if let Some(expected) = case[field].as_str()
                        && got.get(field) != expected
                    {
                        failures.push(format!(
                            "{input:?} base {}: {field} is {:?}, expected {expected:?}",
                            case["base"],
                            got.get(field)
                        ));
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {}:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

/// Runs host cases the way web-platform-tests' own scripts
/// (`toascii.window.js`, `IdnaTestV2.any.js`) do, through
/// `https://<input>/x`: an empty input cannot be tested that way, and
/// they skip it.
fn host_cases(name: &str, expected_count: usize) {
    let cases = load(name);
    let run: Vec<&Value> = cases.iter().filter(|case| case["input"] != "").collect();
    assert_eq!(run.len(), expected_count, "every case is run");
    let mut failures = Vec::new();
    for case in &run {
        let input = case["input"].as_str().expect("an input");
        let parsed = Url::parse(&format!("https://{input}/x"));
        match (parsed, case["output"].as_str()) {
            (Err(_), None) => {}
            (Ok(url), None) => {
                failures.push(format!("{input:?} parsed as {url}, expected failure"))
            }
            (Err(e), Some(expected)) => {
                failures.push(format!("{input:?} failed ({e:?}), expected {expected:?}"))
            }
            (Ok(url), Some(expected)) => {
                let got = Getters::of(&url);
                let href = format!("https://{expected}/x");
                if got.host != expected
                    || got.hostname != expected
                    || got.pathname != "/x"
                    || got.href != href
                {
                    failures.push(format!("{input:?}: {:?}, expected {href:?}", got.href));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {}:\n{}",
        failures.len(),
        run.len(),
        failures.join("\n")
    );
}

#[test]
fn toascii() {
    host_cases("toascii.json", 87);
}

#[test]
fn idna_test_v2() {
    host_cases("IdnaTestV2.json", 2670);
}

#[test]
fn percent_encoding_in_a_documents_encoding() {
    let cases = load("percent-encoding.json");
    assert_eq!(cases.len(), 7);
    let mut run = 0;
    let mut skipped = Vec::new();
    for case in &cases {
        let input = case["input"].as_str().expect("an input");
        let outputs = case["output"].as_object().expect("outputs by encoding");
        let utf8 = outputs["utf-8"].as_str().expect("a UTF-8 output");
        for (label, expected) in outputs {
            let encoding = encoding::Encoding::for_label(label).expect("a label");
            if !encoding.can_decode() {
                skipped.push(label.as_str());
                continue;
            }
            // As web-platform-tests does with an <a href> in a document
            // of that encoding: the query is in it, the fragment UTF-8.
            let url = Url::parse_with(
                &format!("https://doesnotmatter.invalid/?{input}#{input}"),
                None,
                encoding,
            )
            .expect("parses");
            assert_eq!(url.query(), expected.as_str(), "{label} {input:?}");
            assert_eq!(url.fragment(), Some(utf8), "{label} {input:?}");
            run += 1;
        }
    }
    assert_eq!(run, 9);
    skipped.sort_unstable();
    skipped.dedup();
    assert_eq!(
        skipped,
        ["big5", "euc-kr", "gb18030", "iso-2022-jp", "shift_jis"]
    );
}
