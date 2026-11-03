//! A document parsed as its bytes arrive (M24.8, `docs/design/scripting.md`):
//! the encoding sniffed once the first [`PRESCAN_LENGTH`] bytes are in
//! (or all of them, if fewer), the bytes decoded as they come and fed to
//! the [`Parser`], and the parse started again from the first byte when a
//! `<meta>` changes a tentative encoding (HTML section 13.2.3.4: a reload
//! in place, for which the caller drops the document it built). Asked
//! to, it gives the decoded text to a [`Scanner`] too (M38), whose
//! hints the caller takes.

use dom::Document;
use encoding::{DecoderMode, Encoding, UTF_8};

use crate::encoding_sniffing::{self, Confidence, PRESCAN_LENGTH};
use crate::preload::{Hint, Scanner};
use crate::xml::{self, XmlError, XmlParser, XmlStop};
use crate::{Parser, Stop};

/// What a step of [`Load::parse`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// It parsed, and stopped for this reason.
    Parsed(Stop),
    /// A `<meta>` changed the encoding: the parse starts again, from the
    /// first byte, into a new document, which the caller makes.
    Restarted,
}

/// The bytes decoded and parsed so far, in one encoding.
struct Decoding {
    /// The encoding the bytes are in, as found.
    encoding: Encoding,
    /// The encoding they are decoded with: `encoding`, or a byte order
    /// mark's, or UTF-8 for an encoding this engine cannot decode yet.
    decoded_with: Encoding,
    confidence: Confidence,
    decoder: encoding::Decoder,
    /// How many of the bytes the decoder has been given.
    fed: usize,
    parser: Parser,
    scanner: Option<Scanner>,
}

/// A document's bytes, as they arrive, parsed.
pub struct Load {
    transport: Option<Encoding>,
    scripting: bool,
    /// Every byte so far: a restart decodes them again.
    bytes: Vec<u8>,
    ended: bool,
    decoding: Option<Decoding>,
    /// Whether the text is scanned for hints too (M38), and those
    /// found and not taken, from every start.
    scanning: bool,
    hints: Vec<Hint>,
    /// An XML document's parse, for a load made by [`Load::xml`].
    xml: Option<Box<XmlLoad>>,
    /// The parse was aborted: it is done, and takes nothing more.
    aborted: bool,
    /// Whether its parser attaches declarative shadow roots (D68).
    declarative_shadow_roots: bool,
}

/// An XML document's parse: its bytes taken whole, then parsed a slice
/// at a time by the XML parser, its scripts run as it closes them.
struct XmlLoad {
    /// Made once the bytes have all come.
    parser: Option<XmlParser>,
    /// What the bytes were decoded with.
    encoding: Encoding,
    /// The error the parse stopped at, if one.
    error: Option<XmlError>,
}

impl Load {
    /// A load whose transport layer names `transport` as the encoding
    /// (the `Content-Type` charset), its parser's scripting flag
    /// `scripting`.
    pub fn new(transport: Option<Encoding>, scripting: bool) -> Load {
        Load {
            transport,
            scripting,
            bytes: Vec::new(),
            ended: false,
            decoding: None,
            scanning: false,
            hints: Vec::new(),
            xml: None,
            aborted: false,
            declarative_shadow_roots: true,
        }
    }

    /// Whether its parser attaches declarative shadow roots (HTML's
    /// "allow declarative shadow roots"): a navigation's does, a
    /// response's document (XHR's) does not. Set before the bytes come.
    pub fn set_declarative_shadow_roots(&mut self, allow: bool) {
        self.declarative_shadow_roots = allow;
    }

    /// A load of an XML document (XHTML and the other XML types), its
    /// transport layer naming `transport` as the encoding: parsed by the
    /// XML parser once its bytes have all come, into an XML document.
    pub fn xml(transport: Option<Encoding>) -> Load {
        Load {
            xml: Some(Box::new(XmlLoad {
                parser: None,
                encoding: encoding::UTF_8,
                error: None,
            })),
            ..Load::new(transport, true)
        }
    }

    /// Whether it parses an XML document.
    pub fn is_xml(&self) -> bool {
        self.xml.is_some()
    }

    /// The error an XML document's parse stopped at, if one.
    pub fn xml_error(&self) -> Option<&XmlError> {
        self.xml.as_ref().and_then(|xml| xml.error.as_ref())
    }

    /// Scans the text for hints too (M38), from now on; asked before
    /// the first bytes, from the first.
    pub fn set_scanning(&mut self, on: bool) {
        self.scanning = on;
    }

    /// The hints found since they were last taken: after a restart, the
    /// new encoding's again.
    pub fn take_hints(&mut self) -> Vec<Hint> {
        if let Some(scanner) = self.decoding.as_mut().and_then(|d| d.scanner.as_mut()) {
            self.hints.extend(scanner.take_hints());
        }
        std::mem::take(&mut self.hints)
    }

    /// The next bytes of the document.
    pub fn push(&mut self, bytes: &[u8]) {
        if self.ended {
            return;
        }
        self.bytes.extend_from_slice(bytes);
        self.feed();
    }

    /// The end of the document's bytes.
    pub fn end(&mut self) {
        self.ended = true;
        self.feed();
    }

    /// Aborts the parse (HTML's "abort a parser", as `document.open()`
    /// replaces it): what has not been parsed is thrown away, and so is
    /// what arrives after; the parse is done, its open elements left as
    /// they are.
    pub fn abort(&mut self) {
        self.ended = true;
        self.aborted = true;
        self.bytes = Vec::new();
        self.decoding = None;
        self.xml = None;
    }

    /// Whether the parse was aborted.
    pub fn is_aborted(&self) -> bool {
        self.aborted
    }

    /// The encoding the document is decoded with, once it is known.
    pub fn encoding(&self) -> Option<Encoding> {
        if let Some(xml) = &self.xml {
            return xml.parser.as_ref().map(|_| xml.encoding);
        }
        self.decoding.as_ref().map(|d| d.decoded_with)
    }

    /// The encoding the document is in when this engine cannot decode
    /// it yet (a legacy CJK encoding), which it is read as UTF-8 for.
    pub fn undecodable(&self) -> Option<Encoding> {
        if let Some(xml) = &self.xml {
            return (!xml.encoding.can_decode()).then_some(xml.encoding);
        }
        self.decoding
            .as_ref()
            .filter(|d| !d.encoding.can_decode())
            .map(|d| d.encoding)
    }

    /// How sure the encoding is.
    pub fn confidence(&self) -> Option<Confidence> {
        if self.xml.is_some() {
            return self.encoding().map(|_| Confidence::Certain);
        }
        self.decoding.as_ref().map(|d| d.confidence)
    }

    /// Whether the parse is done.
    pub fn is_done(&self) -> bool {
        if self.aborted {
            return true;
        }
        if let Some(xml) = &self.xml {
            return xml.parser.as_ref().is_some_and(XmlParser::is_done);
        }
        self.decoding.as_ref().is_some_and(|d| d.parser.is_done())
    }

    /// Starts decoding in `encoding`, from the first byte.
    fn start(&mut self, encoding: Encoding, confidence: Confidence) {
        if let Some(scanner) = self.decoding.as_mut().and_then(|d| d.scanner.as_mut()) {
            self.hints.extend(scanner.take_hints());
        }
        let (decoded_with, skip) = match encoding::bom_sniff(&self.bytes) {
            Some((bom, length)) => (bom, length),
            None if encoding.can_decode() => (encoding, 0),
            None => (UTF_8, 0),
        };
        let decoder = decoded_with
            .new_decoder(DecoderMode::Replacement)
            .expect("an encoding that decodes has a decoder");
        self.decoding = Some(Decoding {
            encoding,
            decoded_with,
            confidence,
            decoder,
            fed: skip,
            parser: {
                let mut parser = Parser::new(self.scripting);
                parser.set_declarative_shadow_roots(self.declarative_shadow_roots);
                // A document whose scripts run constructs its custom
                // elements as it parses them (D70).
                parser.set_custom_elements(self.scripting);
                parser
            },
            scanner: self.scanning.then(|| Scanner::new(self.scripting)),
        });
    }

    /// Gives the parser what has arrived and not been given, decoded,
    /// once the encoding can be sniffed.
    fn feed(&mut self) {
        if let Some(xml) = &mut self.xml {
            if self.ended && xml.parser.is_none() {
                let (text, encoding) = xml::decode_xml_with(&self.bytes, self.transport);
                xml.encoding = encoding;
                let mut parser = XmlParser::new(&text);
                // A document whose scripts run constructs its custom
                // elements as it parses them (D70).
                parser.set_custom_elements(self.scripting);
                xml.parser = Some(parser);
                self.bytes = Vec::new();
            }
            return;
        }
        if self.decoding.is_none() {
            if self.bytes.len() < PRESCAN_LENGTH && !self.ended {
                return;
            }
            let (encoding, confidence) = encoding_sniffing::sniff(&self.bytes, self.transport);
            self.start(encoding, confidence);
        }
        let Some(decoding) = &mut self.decoding else {
            return;
        };
        let mut text = String::new();
        let _ = decoding
            .decoder
            .decode(&self.bytes[decoding.fed..], self.ended, &mut text);
        decoding.fed = self.bytes.len();
        decoding.parser.feed(&text);
        if let Some(scanner) = &mut decoding.scanner {
            scanner.feed(&text);
        }
        if self.ended {
            decoding.parser.end();
            if let Some(scanner) = &mut decoding.scanner {
                scanner.end();
            }
        }
    }

    /// The local name and `is` value of the custom element the parse
    /// stopped at [`Stop::Construct`] for, the HTML parser's or the XML
    /// parser's (D70).
    pub fn custom_element(&self) -> Option<(&str, Option<&str>)> {
        if let Some(xml) = &self.xml {
            return xml.parser.as_ref().and_then(XmlParser::custom_element);
        }
        self.decoding
            .as_ref()
            .and_then(|decoding| decoding.parser.custom_element())
    }

    /// The prefix of the qualified name of the tag the parse stopped at
    /// [`Stop::Construct`] for: an XML document's alone may have one.
    pub fn custom_prefix(&self) -> Option<&str> {
        self.xml
            .as_ref()
            .and_then(|xml| xml.parser.as_ref())
            .and_then(XmlParser::custom_prefix)
    }

    /// Gives the parser that stopped at [`Stop::Construct`] the element
    /// constructed for the tag it stopped at.
    pub fn provide_element(&mut self, element: dom::NodeId) {
        if let Some(xml) = &mut self.xml {
            if let Some(parser) = &mut xml.parser {
                parser.provide_element(element);
            }
            return;
        }
        if let Some(decoding) = &mut self.decoding {
            decoding.parser.provide_element(element);
        }
    }

    /// The parser, once decoding has begun: `document.write` and the
    /// insertion point are its (M25). An encoding a write's `<meta>`
    /// declares is acted on at the next [`Load::parse`].
    pub fn parser(&mut self) -> Option<&mut Parser> {
        self.decoding.as_mut().map(|decoding| &mut decoding.parser)
    }

    /// Parses at most `slice` tokens into `document`, which must be the
    /// one this load has built into since it began or last restarted.
    pub fn parse(&mut self, document: &mut Document, slice: usize) -> Progress {
        if self.aborted {
            return Progress::Parsed(Stop::Done);
        }
        if let Some(xml) = &mut self.xml {
            let Some(parser) = &mut xml.parser else {
                return Progress::Parsed(Stop::Input);
            };
            return Progress::Parsed(match parser.parse(document, slice) {
                Ok(XmlStop::Slice) => Stop::Slice,
                Ok(XmlStop::Script { element, line }) => Stop::Script { element, line },
                Ok(XmlStop::Construct) => Stop::Construct,
                Ok(XmlStop::Reactions) => Stop::Reactions,
                Ok(XmlStop::Done) => Stop::Done,
                Err(error) => {
                    xml::insert_error_block(document, &error);
                    xml.error = Some(error);
                    Stop::Done
                }
            });
        }
        let Some(decoding) = &mut self.decoding else {
            return Progress::Parsed(Stop::Input);
        };
        let stop = decoding.parser.parse(document, slice);
        if decoding.confidence == Confidence::Tentative
            && let Some(declared) = decoding.parser.encoding_declaration()
        {
            decoding.confidence = Confidence::Certain;
            if let Some(new) = encoding_sniffing::change_the_encoding(decoding.encoding, declared) {
                self.start(new, Confidence::Certain);
                self.feed();
                return Progress::Restarted;
            }
        }
        Progress::Parsed(stop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dom::Node;

    /// `bytes` loaded in pieces of `size`, parsed after each; the tree,
    /// and how many times the parse started again.
    fn loaded(bytes: &[u8], transport: Option<Encoding>, size: usize) -> (Node, usize, Load) {
        let mut load = Load::new(transport, false);
        let mut document = Document::new();
        let mut restarts = 0;
        let mut parse = |load: &mut Load, document: &mut Document| loop {
            match load.parse(document, 5) {
                Progress::Restarted => {
                    restarts += 1;
                    *document = Document::new();
                }
                Progress::Parsed(Stop::Input | Stop::Done) => return,
                Progress::Parsed(_) => {}
            }
        };
        for piece in bytes.chunks(size.max(1)) {
            load.push(piece);
            parse(&mut load, &mut document);
        }
        load.end();
        parse(&mut load, &mut document);
        assert!(load.is_done());
        (document.to_node(), restarts, load)
    }

    #[test]
    fn the_encoding_waits_for_the_prescan_and_then_bytes_are_parsed_as_they_come() {
        let mut load = Load::new(None, false);
        let mut document = Document::new();
        load.push(b"<p>caf\xC3\xA9");
        assert_eq!(load.encoding(), None, "fewer than 1,024 bytes");
        assert_eq!(
            load.parse(&mut document, 100),
            Progress::Parsed(Stop::Input)
        );
        load.push(&vec![b' '; PRESCAN_LENGTH]);
        assert_eq!(load.encoding(), Some(UTF_8));
        assert_eq!(load.confidence(), Some(Confidence::Tentative));
        assert_eq!(
            load.parse(&mut document, 100),
            Progress::Parsed(Stop::Input)
        );
        assert!(
            document
                .text_content(document.root())
                .starts_with("caf\u{e9}")
        );
        // A short document starts at its end.
        let (node, restarts, load) = loaded(b"<p>hi", None, 2);
        assert_eq!(node.text_content(), "hi");
        assert_eq!(restarts, 0);
        assert_eq!(load.encoding(), Some(UTF_8));
    }

    #[test]
    fn a_late_meta_starts_the_parse_again_whatever_the_pieces() {
        // Past the prescan, a meta naming windows-1252: the bytes before
        // it read as UTF-8 at first, then again in windows-1252.
        let mut bytes = b"<title>t</title><p>".to_vec();
        bytes.extend(vec![b'x'; PRESCAN_LENGTH]);
        bytes.extend_from_slice(b"<meta charset=windows-1252><p>caf\xE9");
        let whole = crate::parse_document_from_bytes(&bytes, None);
        assert_eq!(whole.encoding, encoding::WINDOWS_1252);
        for size in [1, 7, 512, 4096] {
            let (node, restarts, load) = loaded(&bytes, None, size);
            assert_eq!(node, whole.document.to_node(), "pieces of {size}");
            assert_eq!(restarts, 1, "pieces of {size}");
            assert_eq!(load.encoding(), Some(encoding::WINDOWS_1252));
            assert_eq!(load.confidence(), Some(Confidence::Certain));
        }
        // The transport's encoding is certain: no restart.
        let (_, restarts, load) = loaded(&bytes, Some(UTF_8), 64);
        assert_eq!(restarts, 0);
        assert_eq!(load.encoding(), Some(UTF_8));
    }

    #[test]
    fn a_byte_order_mark_decides_and_is_not_text() {
        let (node, _, load) = loaded(b"\xEF\xBB\xBF<p>a", Some(encoding::WINDOWS_1252), 1);
        assert_eq!(node.text_content(), "a");
        assert_eq!(load.encoding(), Some(UTF_8));
        assert_eq!(load.confidence(), Some(Confidence::Certain));
    }

    #[test]
    fn an_xml_load_parses_once_its_bytes_have_all_come_and_stops_at_scripts() {
        let mut load = Load::xml(None);
        let mut document = Document::new();
        load.push(b"<html xmlns='http://www.w3.org/1999/xhtml'><script>a</script>");
        assert_eq!(
            load.parse(&mut document, 100),
            Progress::Parsed(Stop::Input)
        );
        assert_eq!(load.encoding(), None);
        load.push(b"<p>&#233;</p></html>");
        load.end();
        assert!(load.is_xml());
        assert_eq!(load.encoding(), Some(UTF_8));
        assert!(load.parser().is_none(), "no HTML parser to write to");
        assert!(matches!(
            load.parse(&mut document, 100),
            Progress::Parsed(Stop::Script { .. })
        ));
        assert_eq!(document.text_content(document.root()), "a");
        assert_eq!(load.parse(&mut document, 100), Progress::Parsed(Stop::Done));
        assert!(load.is_done());
        assert_eq!(document.text_content(document.root()), "a\u{e9}");
        // An error is shown above what came before it.
        let mut load = Load::xml(None);
        let mut document = Document::new();
        load.push(b"<html xmlns='http://www.w3.org/1999/xhtml'><p>a</q></html>");
        load.end();
        assert_eq!(load.parse(&mut document, 100), Progress::Parsed(Stop::Done));
        assert!(load.xml_error().is_some());
        assert!(
            document
                .text_content(document.root())
                .starts_with("This page contains the following errors:")
        );
    }

    #[test]
    fn an_aborted_load_parses_nothing_more() {
        let mut load = Load::new(Some(UTF_8), true);
        let mut document = Document::new();
        load.push(b"<p>a");
        load.push(&vec![b' '; PRESCAN_LENGTH]);
        assert_eq!(
            load.parse(&mut document, 100),
            Progress::Parsed(Stop::Input)
        );
        load.abort();
        load.push(b"<p>b");
        assert!(load.is_aborted() && load.is_done());
        assert_eq!(load.parse(&mut document, 100), Progress::Parsed(Stop::Done));
        assert_eq!(document.text_content(document.root()).trim(), "a");
    }

    #[test]
    fn a_scanning_load_hints_at_what_arrives_before_it_is_parsed() {
        let mut load = Load::new(Some(UTF_8), true);
        load.set_scanning(true);
        load.push(b"<script src=a.js></script><img src=b.png>");
        load.end();
        let urls: Vec<String> = load.take_hints().into_iter().map(|h| h.url).collect();
        assert_eq!(urls, ["a.js", "b.png"], "nothing parsed yet");
        assert!(load.take_hints().is_empty());
        // A restart scans again; the caller keeps one request a URL.
        let mut bytes = b"<img src=x.png>".to_vec();
        bytes.extend(vec![b' '; PRESCAN_LENGTH]);
        bytes.extend_from_slice(b"<meta charset=windows-1252>");
        let mut load = Load::new(None, false);
        load.set_scanning(true);
        load.push(&bytes);
        load.end();
        let mut document = Document::new();
        while load.parse(&mut document, 100) != Progress::Restarted {}
        let urls: Vec<String> = load.take_hints().into_iter().map(|h| h.url).collect();
        assert_eq!(urls, ["x.png", "x.png"]);
        // Not asked: none.
        let mut load = Load::new(None, false);
        load.push(b"<img src=x.png>");
        load.end();
        assert!(load.take_hints().is_empty());
    }
}
