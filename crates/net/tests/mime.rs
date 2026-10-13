//! MIME types and `data:` URLs against web-platform-tests' files in
//! `data/` (see `data/README.md`), run as the scripts beside them run
//! them. Every case is run, and the counts are checked so that none is
//! skipped without saying so.

#![forbid(unsafe_code)]

mod support;

use encoding::UTF_8;
use net::mime::{MimeType, extract_mime_type, legacy_extract_encoding};
use net::{HeaderList, Url, data_url};
use serde_json::Value;

/// What web-platform-tests' `isByteCompatible` says of a type as a
/// header value.
#[derive(Debug, PartialEq)]
enum ByteCompatible {
    /// Usable as it is.
    Yes,
    /// Leading or trailing HTTP whitespace, which `Headers` would strip:
    /// not tried as a header.
    Whitespace,
    /// A code point past U+00FF, or NUL, LF or CR, which `Headers`
    /// refuses: not tried as a header.
    No,
}

fn byte_compatible(input: &str) -> ByteCompatible {
    let http_whitespace = |c: char| matches!(c, '\t' | ' ' | '\n' | '\r');
    if input.starts_with(http_whitespace) || input.ends_with(http_whitespace) {
        return ByteCompatible::Whitespace;
    }
    if input
        .chars()
        .any(|c| u32::from(c) > 0xFF || matches!(c, '\0' | '\n' | '\r'))
    {
        return ByteCompatible::No;
    }
    ByteCompatible::Yes
}

/// Inputs whose `output` is what parsing gives but not what Fetch's
/// extraction gives, with what it gives. Extraction splits the header
/// at each comma outside a quoted string before parsing (Fetch §3.5,
/// *get, decode, and split*), so the type here is what comes before the
/// comma. `parsing.any.js` expects `output` from a `Request`'s and a
/// `Response`'s `Content-Type` too, which Fetch does not give.
const SPLIT_AT_A_COMMA: [(&str, &str); 2] =
    [("x/x;,=x;bonus=x", "x/x"), ("x/x;x=,;bonus=x", "x/x")];

/// The cases of one MIME type file, run as `parsing.any.js` and
/// `charset-parameter.window.js` run them:
///
/// - every input is parsed and serialized, giving `output` or failure;
/// - a byte-compatible input is also a response's `Content-Type`, and
///   the type extracted from it must serialize the same, but for
///   [`SPLIT_AT_A_COMMA`];
/// - where the case gives the `encoding` a document served with it
///   would have, the extracted type's `charset` must name it, with
///   UTF-8 when it names none.
///
/// Returns how many cases went through the header, how many of those
/// were split at a comma, and how many checked an encoding.
fn mime_type_cases(name: &str, expected_count: usize) -> (usize, usize, usize) {
    let cases = support::objects(name);
    assert_eq!(cases.len(), expected_count, "every case is run");
    let mut failures = Vec::new();
    let (mut as_header, mut split_cases, mut encodings) = (0, 0, 0);
    for case in &cases {
        let input = case["input"].as_str().expect("an input");
        let expected = case["output"].as_str();
        let parsed = MimeType::parse(input);
        if parsed.as_ref().map(MimeType::to_string).as_deref() != expected {
            failures.push(format!(
                "{input:?} parsed as {parsed:?}, expected {expected:?}"
            ));
        }
        if byte_compatible(input) != ByteCompatible::Yes {
            continue;
        }
        let mut headers = HeaderList::new();
        let bytes: Vec<u8> = input.chars().map(|c| c as u8).collect();
        headers.append(b"Content-Type", &bytes);
        let extracted = extract_mime_type(&headers);
        as_header += 1;
        let expected = match SPLIT_AT_A_COMMA.iter().find(|(i, _)| *i == input) {
            Some((_, split)) => {
                split_cases += 1;
                Some(*split)
            }
            None => expected,
        };
        if extracted.as_ref().map(MimeType::to_string).as_deref() != expected {
            failures.push(format!(
                "{input:?} as a header gave {extracted:?}, expected {expected:?}"
            ));
        }
        if case.get("navigable").is_some()
            && let Some(encoding) = case.get("encoding")
        {
            encodings += 1;
            let got = legacy_extract_encoding(extracted.as_ref(), UTF_8).name();
            let want = encoding.as_str().unwrap_or("UTF-8");
            if got != want {
                failures.push(format!("{input:?}: encoding {got}, expected {want}"));
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
    (as_header, split_cases, encodings)
}

#[test]
fn mime_types() {
    assert_eq!(mime_type_cases("mime-types.json", 74), (65, 0, 40));
}

#[test]
fn generated_mime_types() {
    assert_eq!(
        mime_type_cases("generated-mime-types.json", 881),
        (862, SPLIT_AT_A_COMMA.len(), 0)
    );
}

/// A `data:` URL fetched as `fetch()` would: a URL that does not parse,
/// or one the processor fails on, is a network error.
fn fetch(input: &str) -> Option<(String, Vec<u8>)> {
    let url = Url::parse(input).ok()?;
    let data = data_url::process(&url)?;
    Some((data.mime_type.to_string(), data.body))
}

fn bytes(value: &Value) -> Vec<u8> {
    value
        .as_array()
        .expect("an array of bytes")
        .iter()
        .map(|b| u8::try_from(b.as_u64().expect("a byte")).expect("a byte"))
        .collect()
}

#[test]
fn data_urls() {
    // [input, type or null for a network error, body]
    let cases = support::read("data-urls.json");
    let cases = cases.as_array().expect("an array");
    assert_eq!(cases.len(), 72, "every case is run");
    let mut failures = Vec::new();
    for case in cases {
        let input = case[0].as_str().expect("an input");
        let expected = case[1]
            .as_str()
            .map(|mime_type| (mime_type.to_owned(), bytes(&case[2])));
        let got = fetch(input);
        if got != expected {
            failures.push(format!("{input:?}: {got:?}, expected {expected:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn base64_in_data_urls() {
    // [input, body or null for a network error], fetched as
    // `data:;base64,<input>`.
    let cases = support::read("base64.json");
    let cases = cases.as_array().expect("an array");
    assert_eq!(cases.len(), 80, "every case is run");
    let mut failures = Vec::new();
    for case in cases {
        let input = case[0].as_str().expect("an input");
        let expected = (!case[1].is_null()).then(|| bytes(&case[1]));
        let got = fetch(&format!("data:;base64,{input}")).map(|(_, body)| body);
        if got != expected {
            failures.push(format!("{input:?}: {got:?}, expected {expected:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
