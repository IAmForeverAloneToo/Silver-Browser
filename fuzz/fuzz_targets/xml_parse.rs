//! The XML parser (M32, `docs/design/svg.md`) on arbitrary bytes, as an
//! SVG image's or document's are read: decoded by their byte order mark
//! or declaration, entities expanded within their bounds, namespaces
//! bound. Nothing panics; a document it makes is valid
//! (`Document::validate`) and nests no deeper than `MAX_XML_DEPTH`; an
//! error names a line and a column from 1.

#![no_main]
#![forbid(unsafe_code)]

use html::xml::{MAX_XML_DEPTH, parse_xml};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    match parse_xml(data) {
        Ok(document) => {
            document.validate().expect("the parser's tree is valid");
            let root = document.root();
            let deepest = document
                .subtree(root)
                .map(|node| {
                    let mut depth = 0;
                    let mut at = document.parent(node);
                    while let Some(parent) = at {
                        depth += 1;
                        at = document.parent(parent);
                    }
                    depth
                })
                .max()
                .unwrap_or(0);
            assert!(deepest <= MAX_XML_DEPTH + 1, "{deepest}");
            let _ = document.text_content(root);
        }
        Err(error) => {
            assert!(error.line >= 1 && error.column >= 1, "{error:?}");
            let _ = error.to_string();
        }
    }
});
