//! HTML parsing: source text or bytes in, DOM tree out.
//!
//! [`parse_document`] runs the [`tokenizer`] (HTML section 13.2.5's
//! state machine) into the [`tree_builder`], producing a `dom::Node`
//! tree; the builder says which state each element's content is
//! tokenized in.
//! [`parse_document_from_bytes`] first finds the bytes' character
//! encoding as HTML does ([`encoding_sniffing`]). HTML parsing
//! is defined to always recover, so the parse never panics and currently
//! never fails; the `Result` keeps the layering contract (parsers return
//! `Result`, never panic) and leaves room for strict-mode diagnostics.
//! Parsers here never perform I/O — resource loading is `net` +
//! `engine`'s job.

pub mod encoding_sniffing;
mod named_references;
pub mod tokenizer;
pub mod tree_builder;

pub use encoding_sniffing::Confidence;
pub use tokenizer::{Doctype, TextState, Token, Tokenizer, content_state, tokenize};
pub use tree_builder::TreeBuilder;

use dom::Node;
use encoding::Encoding;

/// Errors produced by HTML parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// Never produced today: HTML parsing always recovers. Reserved for
    /// future strict-mode diagnostics.
    Reserved,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Reserved => write!(f, "reserved for future strict-mode parsing errors"),
        }
    }
}

impl std::error::Error for ParseError {}

/// Parses an HTML document into a DOM tree.
///
/// Any input yields a tree: unclosed elements close at EOF, unmatched
/// end tags are dropped, and malformed markup degrades gracefully (see
/// the [`tokenizer`] and [`tree_builder`] docs for the exact subset).
pub fn parse_document(input: &str) -> Result<Node, ParseError> {
    let mut tokenizer = Tokenizer::new(input);
    let mut builder = TreeBuilder::new();
    while let Some(token) = tokenizer.next_token() {
        if let Some(state) = builder.process(token) {
            tokenizer.set_state(state);
        }
    }
    Ok(builder.finish())
}

/// A document parsed from bytes, and the encoding it was read in.
#[derive(Debug, Clone)]
pub struct DecodedDocument {
    pub document: Node,
    /// The document's character encoding: the one its bytes were
    /// decoded with.
    pub encoding: Encoding,
    /// How sure the parser was of the encoding when it finished.
    pub confidence: Confidence,
    /// The encoding the document turned out to be in, when the
    /// `encoding` crate cannot decode it yet (a legacy CJK encoding):
    /// the bytes were decoded as UTF-8 instead, and `encoding` says so.
    pub undecodable: Option<Encoding>,
}

/// Parses an HTML document from its bytes. `transport` is the encoding
/// the transport layer gives, the `Content-Type` charset, if any.
///
/// The encoding sniffing algorithm picks the encoding to start with
/// (HTML §13.2.3.2). While its confidence is tentative, the first
/// `<meta>` declaring an encoding settles it: the same encoding makes
/// the confidence certain, and another one has the bytes decoded with
/// it and parsed again from the start, certain (§13.2.3.4). So a
/// document is parsed at most twice.
pub fn parse_document_from_bytes(bytes: &[u8], transport: Option<Encoding>) -> DecodedDocument {
    let (mut encoding, mut confidence) = encoding_sniffing::sniff(bytes, transport);
    loop {
        let (text, decoded_with) = encoding::decode(bytes, encoding);
        let mut tokenizer = Tokenizer::new(&text);
        let mut builder = TreeBuilder::new();
        let mut again = None;
        while let Some(token) = tokenizer.next_token() {
            if let Some(state) = builder.process(token) {
                tokenizer.set_state(state);
            }
            if confidence == Confidence::Tentative
                && let Some(declared) = builder.encoding_declaration()
            {
                confidence = Confidence::Certain;
                again = encoding_sniffing::change_the_encoding(encoding, declared);
                if again.is_some() {
                    break;
                }
            }
        }
        match again {
            Some(new) => encoding = new,
            None => {
                return DecodedDocument {
                    document: builder.finish(),
                    encoding: decoded_with,
                    confidence,
                    undecodable: (!encoding.can_decode()).then_some(encoding),
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_small_document() {
        let doc = parse_document(
            "<html><body><h1>Hello from Silver Browser</h1><p>A hobby browser engine in Rust.</p></body></html>",
        )
        .expect("parsing never fails");
        assert_eq!(
            doc.text_content(),
            "Hello from Silver BrowserA hobby browser engine in Rust."
        );
    }

    #[test]
    fn empty_input_yields_an_empty_document() {
        let doc = parse_document("").expect("parsing never fails");
        assert_eq!(doc.text_content(), "");
    }

    #[test]
    fn formatting_whitespace_survives_as_text() {
        let doc = parse_document("<p>a</p>\n<p>b</p>").expect("parsing never fails");
        assert_eq!(doc.text_content(), "a\nb");
    }

    use encoding::{GBK, KOI8_R, UTF_8, WINDOWS_1252};

    /// `head`, then more than the prescan reads, then `tail`: a
    /// declaration in `tail` is one only the parser sees.
    fn past_the_prescan(head: &[u8], tail: &[u8]) -> Vec<u8> {
        let mut bytes = head.to_vec();
        bytes.extend_from_slice(b"<!--");
        bytes.extend(std::iter::repeat_n(b'-', encoding_sniffing::PRESCAN_LENGTH));
        bytes.extend_from_slice(b"-->");
        bytes.extend_from_slice(tail);
        bytes
    }

    fn summary(decoded: &DecodedDocument) -> (String, Encoding, Confidence, Option<Encoding>) {
        (
            decoded.document.text_content(),
            decoded.encoding,
            decoded.confidence,
            decoded.undecodable,
        )
    }

    #[test]
    fn a_meta_the_prescan_found_is_confirmed_by_the_parser() {
        let decoded = parse_document_from_bytes(b"<meta charset=windows-1252><p>caf\xE9", None);
        let expected = ("café".to_owned(), WINDOWS_1252, Confidence::Certain, None);
        assert_eq!(summary(&decoded), expected);
    }

    #[test]
    fn a_late_meta_has_the_bytes_parsed_again() {
        let bytes = past_the_prescan(
            b"<title>caf\xE9</title>",
            b"<meta charset=windows-1252><p>na\xEFve",
        );
        let decoded = parse_document_from_bytes(&bytes, None);
        let expected = (
            "cafénaïve".to_owned(),
            WINDOWS_1252,
            Confidence::Certain,
            None,
        );
        assert_eq!(summary(&decoded), expected);
        // The pragma form, and a <meta> in the body, count the same.
        let bytes = past_the_prescan(
            b"<p>caf\xE9",
            b"<meta http-equiv=content-type content='text/html; charset=latin1'>",
        );
        let decoded = parse_document_from_bytes(&bytes, None);
        assert_eq!(
            summary(&decoded),
            ("café".to_owned(), WINDOWS_1252, Confidence::Certain, None)
        );
    }

    #[test]
    fn only_the_first_declaration_counts() {
        let bytes = past_the_prescan(b"", b"<meta charset=utf-8><meta charset=koi8-r>\xC3\xA9");
        let decoded = parse_document_from_bytes(&bytes, None);
        assert_eq!(
            summary(&decoded),
            ("é".to_owned(), UTF_8, Confidence::Certain, None)
        );
        // A <meta> declaring nothing it can name is passed over.
        let bytes = past_the_prescan(b"", b"<meta charset=bogus><meta charset=koi8-r>\xC1");
        let decoded = parse_document_from_bytes(&bytes, None);
        assert_eq!(
            summary(&decoded),
            ("а".to_owned(), KOI8_R, Confidence::Certain, None)
        );
    }

    #[test]
    fn a_certain_encoding_ignores_meta() {
        let bytes = b"<meta charset=koi8-r>caf\xE9";
        let decoded = parse_document_from_bytes(bytes, Some(WINDOWS_1252));
        assert_eq!(
            summary(&decoded),
            ("café".to_owned(), WINDOWS_1252, Confidence::Certain, None)
        );
        let bytes = b"\xEF\xBB\xBF<meta charset=koi8-r>caf\xC3\xA9";
        let decoded = parse_document_from_bytes(bytes, Some(WINDOWS_1252));
        assert_eq!(
            summary(&decoded),
            ("café".to_owned(), UTF_8, Confidence::Certain, None)
        );
    }

    #[test]
    fn without_a_declaration_utf8_stays_tentative() {
        let decoded = parse_document_from_bytes(b"<p>caf\xC3\xA9 caf\xE9", None);
        let expected = (
            "café caf\u{FFFD}".to_owned(),
            UTF_8,
            Confidence::Tentative,
            None,
        );
        assert_eq!(summary(&decoded), expected);
    }

    #[test]
    fn a_utf16_declaration_in_ascii_bytes_means_utf8() {
        let bytes = past_the_prescan(b"", b"<meta charset=utf-16le>\xC3\xA9");
        let decoded = parse_document_from_bytes(&bytes, None);
        assert_eq!(
            summary(&decoded),
            ("é".to_owned(), UTF_8, Confidence::Certain, None)
        );
    }

    #[test]
    fn an_encoding_not_decoded_yet_is_reported() {
        let bytes = b"<p>a\xC4\xE3b";
        let decoded = parse_document_from_bytes(bytes, Some(GBK));
        let expected = (
            "a\u{FFFD}\u{FFFD}b".to_owned(),
            UTF_8,
            Confidence::Certain,
            Some(GBK),
        );
        assert_eq!(summary(&decoded), expected);
        let bytes = past_the_prescan(b"<p>a\xC4\xE3b", b"<meta charset=gbk>");
        let decoded = parse_document_from_bytes(&bytes, None);
        assert_eq!(summary(&decoded), expected);
    }
}
