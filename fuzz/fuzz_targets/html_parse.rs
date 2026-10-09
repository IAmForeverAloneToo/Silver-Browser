//! The HTML tokenizer and tree builder on arbitrary text, and the tree
//! walked once afterwards (a tree the parser builds must also survive
//! being read and dropped). Any bytes also go through the encoding
//! sniffing algorithm, its prescan, and the parse again that a late
//! `<meta>` asks for: the encoding a document is decoded with is always
//! one the `encoding` crate decodes, UTF-8 when the one found is not.
//! Text is also parsed fed in pieces, a few tokens at a time and on
//! past each script's end tag, as a page loading is (M24.8): the tree is
//! the one the whole text gives; and with the scripting flag, which
//! never panics either.

#![no_main]

use html::{Parser, Stop};
use libfuzzer_sys::fuzz_target;

/// Parses what `parser` has been given, a few tokens at a time.
fn drain(parser: &mut Parser, document: &mut dom::Document) -> Stop {
    loop {
        match parser.parse(document, 7) {
            Stop::Slice | Stop::Script { .. } => {}
            stop => return stop,
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let decoded = html::parse_document_from_bytes(data, None);
    assert!(decoded.encoding.can_decode(), "{:?}", decoded.encoding);
    if let Some(undecodable) = decoded.undecodable {
        assert!(!undecodable.can_decode(), "{undecodable:?}");
        assert_eq!(decoded.encoding, encoding::UTF_8);
    }
    let _ = decoded.document.text_content(decoded.document.root());
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(whole) = html::parse_document(s) else {
        return;
    };
    let _ = whole.text_content();
    let size = 1 + usize::from(data.first().copied().unwrap_or(0)) % 17;
    let chars: Vec<char> = s.chars().collect();
    for scripting in [false, true] {
        let mut parser = Parser::new(scripting);
        let mut document = dom::Document::new();
        for piece in chars.chunks(size) {
            parser.feed(&piece.iter().collect::<String>());
            assert_eq!(drain(&mut parser, &mut document), Stop::Input);
        }
        parser.end();
        assert_eq!(drain(&mut parser, &mut document), Stop::Done);
        if !scripting {
            assert_eq!(document.to_node(), whole, "pieces of {size}");
        }
    }
});
