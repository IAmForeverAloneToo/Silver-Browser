//! Reading the web-platform-tests files in `data/` (see
//! `data/README.md`).

use std::path::Path;

use serde_json::Value;

/// The JSON file `name` in `data/`.
pub fn read(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let text = lone_surrogates_replaced(&text);
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// The objects of the JSON array in `name`; the strings between them
/// are comments.
pub fn objects(name: &str) -> Vec<Value> {
    read(name)
        .as_array()
        .expect("an array")
        .iter()
        .filter(|entry| entry.is_object())
        .cloned()
        .collect()
}

/// The JSON text with each `\uXXXX` escape of a lone surrogate written
/// as `�`. JavaScript hands such a string to `new URL()` through
/// WebIDL's USVString conversion, which makes the same replacement; a
/// Rust string cannot hold a lone surrogate at all.
fn lone_surrogates_replaced(text: &str) -> String {
    let escape_at = |i: usize| -> Option<u16> {
        let hex = text.get(i..i + 6)?.strip_prefix("\\u")?;
        u16::from_str_radix(hex, 16).ok()
    };
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        // An escaped backslash is not the start of an escape.
        if text[i..].starts_with("\\\\") {
            out.push_str("\\\\");
            i += 2;
            continue;
        }
        match escape_at(i) {
            // A high surrogate followed by a low one: a valid pair.
            Some(0xD800..=0xDBFF) if matches!(escape_at(i + 6), Some(0xDC00..=0xDFFF)) => {
                out.push_str(&text[i..i + 12]);
                i += 12;
            }
            Some(0xD800..=0xDFFF) => {
                out.push_str("\\uFFFD");
                i += 6;
            }
            _ => {
                let c = text[i..].chars().next().expect("not at the end");
                out.push(c);
                i += c.len_utf8();
            }
        }
    }
    out
}
