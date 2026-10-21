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
        }
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

    /// The encoding the document is decoded with, once it is known.
    pub fn encoding(&self) -> Option<Encoding> {
        self.decoding.as_ref().map(|d| d.decoded_with)
    }

    /// The encoding the document is in when this engine cannot decode
    /// it yet (a legacy CJK encoding), which it is read as UTF-8 for.
    pub fn undecodable(&self) -> Option<Encoding> {
        self.decoding
            .as_ref()
            .filter(|d| !d.encoding.can_decode())
            .map(|d| d.encoding)
    }

    /// How sure the encoding is.
    pub fn confidence(&self) -> Option<Confidence> {
        self.decoding.as_ref().map(|d| d.confidence)
    }

    /// Whether the parse is done.
    pub fn is_done(&self) -> bool {
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
            parser: Parser::new(self.scripting),
            scanner: self.scanning.then(|| Scanner::new(self.scripting)),
        });
    }

    /// Gives the parser what has arrived and not been given, decoded,
    /// once the encoding can be sniffed.
    fn feed(&mut self) {
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

    /// The parser, once decoding has begun: `document.write` and the
    /// insertion point are its (M25). An encoding a write's `<meta>`
    /// declares is acted on at the next [`Load::parse`].
    pub fn parser(&mut self) -> Option<&mut Parser> {
        self.decoding.as_mut().map(|decoding| &mut decoding.parser)
    }

    /// Parses at most `slice` tokens into `document`, which must be the
    /// one this load has built into since it began or last restarted.
    pub fn parse(&mut self, document: &mut Document, slice: usize) -> Progress {
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
