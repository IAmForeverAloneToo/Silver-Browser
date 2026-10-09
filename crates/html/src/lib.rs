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
mod load;
mod named_references;
mod serialize;
pub mod tokenizer;
pub mod tree_builder;

pub use encoding_sniffing::Confidence;
pub use load::{Load, Progress};
pub use serialize::{serialize_children, serialize_node};
pub use tokenizer::{Doctype, TextState, Token, Tokenizer, content_state, tokenize};
pub use tree_builder::TreeBuilder;

use dom::{Document, Node, NodeId};
use encoding::Encoding;

/// Why [`Parser::parse`] stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// The input given so far is used up; more is to come.
    Input,
    /// It processed the tokens it was allowed.
    Slice,
    /// A `script` element's end tag was processed: the caller prepares
    /// the script before it parses on (HTML section 13.2.6.4.8), and a
    /// parser-blocking one before it parses at all. `line` is the line
    /// the script's text began on.
    Script { element: NodeId, line: u32 },
    /// The input is parsed to its end and the tree finished.
    Done,
}

/// An HTML parser (M24.8): input fed in pieces, as it is decoded, and
/// tokens processed into a [`dom::Document`] a slice at a time, the tree
/// growing as they are, the parse stopping at each script's end tag.
pub struct Parser {
    tokenizer: Tokenizer,
    builder: TreeBuilder,
    done: bool,
}

impl Parser {
    /// A parser for a fresh document, with HTML's scripting flag set if
    /// `scripting` (section 13.2.4.5): then `noscript`'s content is text.
    pub fn new(scripting: bool) -> Parser {
        Parser {
            tokenizer: Tokenizer::streaming(),
            builder: TreeBuilder::new(scripting),
            done: false,
        }
    }

    /// A parser for a fragment (HTML section 13.4) in the context of an
    /// element with local name `context`, building into the children of
    /// `container` (see [`TreeBuilder::fragment`]). The tokenizer starts
    /// in the state the context's content is read in; no start tag has
    /// been emitted, so no end tag ends that state (`a</textarea>b` in a
    /// `textarea` is all text).
    pub fn fragment(container: NodeId, context: &str, scripting: bool, max_depth: usize) -> Parser {
        let mut tokenizer = Tokenizer::streaming();
        let state = if context == "noscript" && scripting {
            Some(TextState::Rawtext)
        } else {
            content_state(context)
        };
        if let Some(state) = state {
            tokenizer.set_state(state);
        }
        Parser {
            tokenizer,
            builder: TreeBuilder::fragment(container, context, scripting, max_depth),
            done: false,
        }
    }

    /// Appends `text` to the input.
    pub fn feed(&mut self, text: &str) {
        self.tokenizer.feed(text);
    }

    /// Marks the end of the input.
    pub fn end(&mut self) {
        self.tokenizer.end();
    }

    /// Whether the parse is done.
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// The encoding the first `<meta>` parsed declares, if any.
    pub fn encoding_declaration(&self) -> Option<Encoding> {
        self.builder.encoding_declaration()
    }

    /// Processes at most `slice` tokens into `document`, which must be
    /// the one this parser has built into from the start.
    pub fn parse(&mut self, document: &mut Document, slice: usize) -> Stop {
        if self.done {
            return Stop::Done;
        }
        for _ in 0..slice {
            let Some(token) = self.tokenizer.next_token() else {
                if !self.tokenizer.is_finished() {
                    return Stop::Input;
                }
                self.builder.finish(document);
                self.done = true;
                return Stop::Done;
            };
            let line = self.tokenizer.line();
            if let Some(state) = self.builder.process(document, token, line) {
                self.tokenizer.set_state(state);
            }
            if let Some((element, line)) = self.builder.take_script() {
                return Stop::Script { element, line };
            }
        }
        Stop::Slice
    }
}

/// Parses the whole of `input` into `document`, running no scripts.
fn parse_whole(parser: &mut Parser, document: &mut Document) {
    while parser.parse(document, usize::MAX) != Stop::Done {}
}

/// Parses `markup` as a fragment (HTML section 13.4, M25's `innerHTML`)
/// for a context element with local name `context`, into the children
/// of `container`, a detached element of `document` that its caller
/// made and moves them out of. At most `max_depth` elements nest, the
/// container counted. Its scripts are the caller's to mark as already
/// started: this runs none.
pub fn parse_fragment(
    document: &mut Document,
    container: NodeId,
    context: &str,
    scripting: bool,
    max_depth: usize,
    markup: &str,
) {
    let mut parser = Parser::fragment(container, context, scripting, max_depth);
    parser.feed(markup);
    parser.end();
    parse_whole(&mut parser, document);
}

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
    let mut parser = Parser::new(false);
    parser.feed(input);
    parser.end();
    let mut document = Document::new();
    parse_whole(&mut parser, &mut document);
    Ok(document.to_node())
}

/// A document parsed from bytes, and the encoding it was read in.
#[derive(Debug, Clone)]
pub struct DecodedDocument {
    pub document: Document,
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
    let mut load = Load::new(transport, false);
    load.push(bytes);
    load.end();
    let mut document = Document::new();
    loop {
        match load.parse(&mut document, usize::MAX) {
            Progress::Parsed(Stop::Done) => break,
            Progress::Parsed(_) => {}
            Progress::Restarted => document = Document::new(),
        }
    }
    DecodedDocument {
        document,
        encoding: load.encoding().unwrap_or(encoding::UTF_8),
        confidence: load.confidence().unwrap_or(Confidence::Certain),
        undecodable: load.undecodable(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `markup` parsed as a fragment for `context`: the container's
    /// markup.
    fn fragment(context: &str, markup: &str) -> String {
        let mut document = Document::new();
        let container = document.create_element("html").unwrap();
        parse_fragment(
            &mut document,
            container,
            context,
            true,
            dom::MAX_DEPTH,
            markup,
        );
        document.validate().unwrap();
        serialize_children(&document, container, true)
    }

    #[test]
    fn a_fragment_parses_as_the_reference_browser_parses_it() {
        assert_eq!(
            fragment("div", "<p>a<p>b</p><b>c"),
            "<p>a</p><p>b</p><b>c</b>"
        );
        // The context's content state, ended by no end tag.
        assert_eq!(
            fragment("textarea", "a</textarea>b<i>"),
            "a&lt;/textarea&gt;b&lt;i&gt;"
        );
        assert_eq!(
            fragment("title", "<b>&amp;</b>"),
            "&lt;b&gt;&amp;&lt;/b&gt;"
        );
        assert_eq!(
            fragment("noscript", "<b>&amp;</b>"),
            "&lt;b&gt;&amp;amp;&lt;/b&gt;"
        );
        // The skeleton's tags are dropped in a body context and made for
        // an `html` one.
        assert_eq!(
            fragment(
                "div",
                "<title>t</title><meta charset=x><body class=q><p>x</p></body><html lang=en>"
            ),
            "<title>t</title><meta charset=\"x\"><p>x</p>"
        );
        assert_eq!(
            fragment("html", "<p>x"),
            "<head></head><body><p>x</p></body>"
        );
        // Scripts are parsed, and not run.
        assert_eq!(
            fragment("div", "<script>x()</script><i>y"),
            "<script>x()</script><i>y</i>"
        );
    }

    #[test]
    fn a_fragment_nests_no_deeper_than_it_is_let() {
        let mut document = Document::new();
        let container = document.create_element("html").unwrap();
        parse_fragment(&mut document, container, "div", false, 4, &"<b>".repeat(10));
        let depth = document
            .subtree(container)
            .map(|n| document.ancestors(n).count())
            .max()
            .unwrap();
        assert_eq!(depth, 3, "the container and three elements down");
        // Even a budget of one keeps the container open.
        let container = document.create_element("html").unwrap();
        parse_fragment(&mut document, container, "div", false, 1, "<b><i>x");
        assert_eq!(document.children(container).count(), 2);
        assert!(
            document
                .children(container)
                .all(|c| document.parent(c) == Some(container))
        );
        document.validate().unwrap();
    }

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
            decoded.document.text_content(decoded.document.root()),
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

    /// The tags of `document`'s elements in tree order.
    fn tags_of(document: &Document) -> Vec<String> {
        document
            .subtree(document.root())
            .filter_map(|n| document.tag(n).map(str::to_owned))
            .collect()
    }

    #[test]
    fn the_parser_stops_after_each_script_with_the_tree_as_it_stands() {
        let mut parser = Parser::new(false);
        parser.feed("<p>a</p>\n<script>\nx()\n</script><p>b</p><script></script>");
        parser.end();
        let mut document = Document::new();
        let Stop::Script { element, line } = parser.parse(&mut document, usize::MAX) else {
            panic!("a script's end tag");
        };
        assert_eq!(document.tag(element), Some("script"));
        assert_eq!(line, 2, "its text starts on the second line");
        assert_eq!(document.text_content(element), "\nx()\n");
        // The tree as it stands: the script in the body, nothing after it.
        assert_eq!(tags_of(&document), ["html", "head", "body", "p", "script"]);
        let body = document.parent(element).unwrap();
        assert_eq!(document.tag(body), Some("body"));
        let Stop::Script {
            element: second, ..
        } = parser.parse(&mut document, usize::MAX)
        else {
            panic!("the second script");
        };
        assert_ne!(second, element);
        assert_eq!(parser.parse(&mut document, usize::MAX), Stop::Done);
        assert!(parser.is_done());
        assert_eq!(
            tags_of(&document),
            ["html", "head", "body", "p", "script", "p", "script"]
        );
        assert_eq!(parser.parse(&mut document, 5), Stop::Done);
    }

    #[test]
    fn a_script_the_input_ends_in_or_in_svg_is_no_stop() {
        for source in ["<script>never closed", "<svg><script>x</script></svg>"] {
            let mut parser = Parser::new(false);
            parser.feed(source);
            parser.end();
            let mut document = Document::new();
            assert_eq!(
                parser.parse(&mut document, usize::MAX),
                Stop::Done,
                "{source}"
            );
        }
    }

    #[test]
    fn the_parser_takes_its_input_in_pieces_and_a_slice_at_a_time() {
        let source = "<!DOCTYPE html><title>t &amp; u</title><p class=a>one<b>two</b>three\
                      <noscript><i>n</i></noscript><textarea>\nx</textarea>";
        let whole = parse_document(source).unwrap();
        let mut parser = Parser::new(false);
        parser.feed(source);
        let mut document = Document::new();
        assert_eq!(parser.parse(&mut document, 1), Stop::Slice);
        assert!(tags_of(&document).is_empty(), "the doctype alone");
        assert_eq!(parser.parse(&mut document, 1), Stop::Slice);
        assert_eq!(tags_of(&document), ["html", "head", "title"]);
        for size in [1, 3, 8] {
            let mut parser = Parser::new(false);
            let mut document = Document::new();
            let chars: Vec<char> = source.chars().collect();
            for piece in chars.chunks(size) {
                parser.feed(&piece.iter().collect::<String>());
                assert_eq!(parser.parse(&mut document, usize::MAX), Stop::Input);
            }
            parser.end();
            // One token a call.
            let mut slices = 0;
            while parser.parse(&mut document, 1) == Stop::Slice {
                slices += 1;
            }
            assert!(slices <= 2, "{slices}");
            assert_eq!(document.to_node(), whole, "pieces of {size}");
        }
    }

    #[test]
    fn with_scripting_noscript_holds_text() {
        let source = "<noscript><p>shown without scripts</p></noscript>";
        let without = parse_document(source).unwrap();
        assert!(format!("{without:?}").contains("tag: \"p\""));
        let mut parser = Parser::new(true);
        parser.feed(source);
        parser.end();
        let mut document = Document::new();
        assert_eq!(parser.parse(&mut document, usize::MAX), Stop::Done);
        let noscript = document
            .subtree(document.root())
            .find(|&n| document.tag(n) == Some("noscript"))
            .unwrap();
        let children: Vec<_> = document.children(noscript).collect();
        assert_eq!(children.len(), 1);
        assert_eq!(
            document.text(children[0]),
            Some("<p>shown without scripts</p>")
        );
    }
}
