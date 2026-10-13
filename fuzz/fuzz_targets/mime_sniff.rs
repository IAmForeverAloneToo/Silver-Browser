//! MIME types and sniffing on arbitrary bytes. A type that parses
//! serializes to one that parses back the same, alone and as extracted
//! from `Content-Type` fields. Sniffing never panics, and never gives a
//! scriptable type where scriptable types are not to be sniffed. `data:`
//! URLs and forgiving-base64 come along: neither panics, and base64
//! never gives more than three bytes for four characters.

#![no_main]
#![forbid(unsafe_code)]

use libfuzzer_sys::fuzz_target;
use net::mime::{MimeType, extract_mime_type, sniff};
use net::{HeaderList, Url, data_url};

fn round_trips(mime_type: &MimeType) {
    let bytes = mime_type.to_bytes();
    let again = MimeType::parse_bytes(&bytes).expect("a serialization parses");
    assert_eq!(&again, mime_type);
    assert_eq!(again.to_bytes(), bytes);
}

fuzz_target!(|data: &[u8]| {
    // The first byte sets the sniffing flags; the rest is the input.
    let Some((&flags, input)) = data.split_first() else {
        return;
    };

    if let Some(mime_type) = MimeType::parse_bytes(input) {
        round_trips(&mime_type);
    }

    // One `Content-Type` field per line: a field value holds no LF.
    let mut headers = HeaderList::new();
    for value in input.split(|&b| b == b'\n') {
        headers.append(b"Content-Type", value);
    }
    let extracted = extract_mime_type(&headers);
    if let Some(mime_type) = &extracted {
        assert_ne!(mime_type.essence(), "*/*");
        round_trips(mime_type);
    }

    let check_for_apache_bug = flags & 1 != 0;
    let no_sniff = flags & 2 != 0;
    let supplied = if flags & 4 != 0 {
        extracted.as_ref()
    } else {
        None
    };
    let computed = sniff::computed_type(supplied, check_for_apache_bug, no_sniff, input);
    if supplied.is_none() && no_sniff {
        assert!(!computed.is_scriptable(), "{computed}");
    }
    assert!(!sniff::identify_unknown(input, false).is_scriptable());
    assert!(!sniff::text_or_binary(input).is_scriptable());

    if let Ok(text) = std::str::from_utf8(input) {
        if let Ok(url) = Url::parse(&format!("data:{text}")) {
            assert_eq!(url.scheme(), "data");
            if let Some(data) = data_url::process(&url) {
                round_trips(&data.mime_type);
            }
        }
        if let Some(decoded) = data_url::forgiving_base64_decode(text) {
            assert!(decoded.len() * 4 <= text.len() * 3, "{text:?}");
        }
    }
});
