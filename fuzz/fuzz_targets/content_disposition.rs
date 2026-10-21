//! `Content-Disposition` (M37) on arbitrary bytes, and the name it
//! suggests made safe: the parse never panics, and the name a download
//! would be saved under is never empty, never a path, never a device's,
//! holds no control or bidirectional character, starts with no dot and
//! ends with no dot or space, and keeps to its bound.

#![no_main]
#![forbid(unsafe_code)]

use engine::downloads::{MAX_FILE_NAME, safe_name};
use libfuzzer_sys::fuzz_target;

const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

fuzz_target!(|data: &[u8]| {
    let disposition = net::content_disposition(data);
    let url = net::Url::parse("https://example.com/files/download").expect("parses");
    let name = safe_name(disposition.filename.as_deref(), &url, "application/pdf");
    assert!(!name.is_empty());
    assert!(name.len() <= MAX_FILE_NAME, "{name:?}");
    assert!(
        !name.starts_with('.') && !name.ends_with('.') && !name.ends_with(' '),
        "{name:?}"
    );
    for c in name.chars() {
        assert!(
            !matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}' | '<' | '>' | ':' | '"' | '/' | '\\' | '|'
                | '?' | '*' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'),
            "{name:?}"
        );
    }
    let stem = name.split('.').next().unwrap_or_default().trim_end();
    assert!(
        !RESERVED
            .iter()
            .any(|reserved| reserved.eq_ignore_ascii_case(stem)),
        "{name:?}"
    );
});
