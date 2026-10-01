//! The HTML tokenizer and tree builder on arbitrary text, and the tree
//! walked once afterwards (a tree the parser builds must also survive
//! being read and dropped).

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(doc) = html::parse_document(s) {
        let _ = doc.text_content();
    }
});
