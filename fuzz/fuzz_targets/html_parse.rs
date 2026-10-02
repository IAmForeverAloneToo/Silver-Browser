//! The HTML tokenizer and tree builder on arbitrary text, and the tree
//! walked once afterwards (a tree the parser builds must also survive
//! being read and dropped). Any bytes also go through the encoding
//! sniffing algorithm, its prescan, and the parse again that a late
//! `<meta>` asks for: the encoding a document is decoded with is always
//! one the `encoding` crate decodes, UTF-8 when the one found is not.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let decoded = html::parse_document_from_bytes(data, None);
    assert!(decoded.encoding.can_decode(), "{:?}", decoded.encoding);
    if let Some(undecodable) = decoded.undecodable {
        assert!(!undecodable.can_decode(), "{undecodable:?}");
        assert_eq!(decoded.encoding, encoding::UTF_8);
    }
    let _ = decoded.document.text_content();
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(doc) = html::parse_document(s) {
        let _ = doc.text_content();
    }
});
