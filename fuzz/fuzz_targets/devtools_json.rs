//! The DevTools protocol's JSON (M36) on arbitrary text: what parses
//! writes back to a text that parses to the same value, and writing
//! that again gives the same text; nothing parsed nests past the bound;
//! and a message that is a request survives being written again.

#![no_main]
#![forbid(unsafe_code)]

use devtools::{Json, MAX_JSON_DEPTH, Request};
use libfuzzer_sys::fuzz_target;

fn depth(value: &Json) -> usize {
    match value {
        Json::Array(items) => 1 + items.iter().map(depth).max().unwrap_or(0),
        Json::Object(members) => 1 + members.iter().map(|(_, v)| depth(v)).max().unwrap_or(0),
        _ => 0,
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(value) = Json::parse(text) {
        assert!(depth(&value) <= MAX_JSON_DEPTH);
        let written = value.write();
        let again = Json::parse(&written)
            .unwrap_or_else(|e| panic!("{written:?} does not parse again: {e}"));
        assert_eq!(again, value, "{written:?}");
        assert_eq!(again.write(), written);
    }
    if let Ok(request) = Request::parse(text) {
        assert_eq!(Request::parse(&request.write()), Ok(request));
    }
});
