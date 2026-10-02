//! The WHATWG Encoding Standard: the web's encodings, their labels, and
//! decoders and encoders written as the standard writes them (roadmap
//! M14, decision D18; see `docs/design/net.md`).
//!
//! A leaf crate. `html` finds a document's encoding with it, `css` a
//! stylesheet's, `net` encodes a URL's query with it, and the engine
//! decodes what it fetches.
//!
//! Of the standard's 40 encodings it has:
//!
//! * every label (228), so every encoding is recognised;
//! * decoders for UTF-8, UTF-16BE and UTF-16LE, the 28 legacy
//!   single-byte encodings, x-user-defined and replacement;
//! * encoders for UTF-8, the single-byte encodings and x-user-defined,
//!   with the standard's fatal and html error modes.
//!
//! The legacy CJK encodings (GBK, gb18030, Big5, EUC-JP, ISO-2022-JP,
//! Shift_JIS and EUC-KR) are recognised but not yet decoded or encoded:
//! [`Encoding::new_decoder`] and [`Encoding::new_encoder`] return `None`
//! for them, and [`decode`] decodes them as UTF-8. They come with M42,
//! or sooner if a page the T1 track measures needs one.
//!
//! A [`Decoder`] is a state machine fed bytes in pieces of any size and
//! told when the input ends, so a sequence split between two pieces
//! decodes as it would whole.
//!
//! The single-byte tables in `tables.rs` are generated from the
//! standard's index files in `data/`, and `tests/standard.rs` checks
//! they still match. The standard licenses the portions of it carried
//! into source code under BSD-3-Clause; see `LICENSE-WHATWG`.

use std::fmt;

mod decoder;
mod encoder;
#[rustfmt::skip]
mod tables;

pub use decoder::{DecodeError, Decoder, DecoderMode};
pub use encoder::{Encoder, EncoderMode, Unmappable};

/// One of the standard's encodings.
///
/// Compared by identity: two values are equal when they are the same
/// encoding. The constants below name all 40, in the standard's order.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Encoding(u8);

/// UTF-8, the encoding (Encoding §4.2).
pub const UTF_8: Encoding = Encoding(0);
/// IBM866, a legacy single-byte encoding.
pub const IBM866: Encoding = Encoding(1);
/// ISO-8859-2, a legacy single-byte encoding.
pub const ISO_8859_2: Encoding = Encoding(2);
/// ISO-8859-3, a legacy single-byte encoding.
pub const ISO_8859_3: Encoding = Encoding(3);
/// ISO-8859-4, a legacy single-byte encoding.
pub const ISO_8859_4: Encoding = Encoding(4);
/// ISO-8859-5, a legacy single-byte encoding.
pub const ISO_8859_5: Encoding = Encoding(5);
/// ISO-8859-6, a legacy single-byte encoding.
pub const ISO_8859_6: Encoding = Encoding(6);
/// ISO-8859-7, a legacy single-byte encoding.
pub const ISO_8859_7: Encoding = Encoding(7);
/// ISO-8859-8, a legacy single-byte encoding.
pub const ISO_8859_8: Encoding = Encoding(8);
/// ISO-8859-8-I: ISO-8859-8's index, named apart because it bears on
/// layout direction (Encoding §9).
pub const ISO_8859_8_I: Encoding = Encoding(9);
/// ISO-8859-10, a legacy single-byte encoding.
pub const ISO_8859_10: Encoding = Encoding(10);
/// ISO-8859-13, a legacy single-byte encoding.
pub const ISO_8859_13: Encoding = Encoding(11);
/// ISO-8859-14, a legacy single-byte encoding.
pub const ISO_8859_14: Encoding = Encoding(12);
/// ISO-8859-15, a legacy single-byte encoding.
pub const ISO_8859_15: Encoding = Encoding(13);
/// ISO-8859-16, a legacy single-byte encoding.
pub const ISO_8859_16: Encoding = Encoding(14);
/// KOI8-R, a legacy single-byte encoding.
pub const KOI8_R: Encoding = Encoding(15);
/// KOI8-U, a legacy single-byte encoding.
pub const KOI8_U: Encoding = Encoding(16);
/// macintosh, a legacy single-byte encoding.
pub const MACINTOSH: Encoding = Encoding(17);
/// windows-874, a legacy single-byte encoding.
pub const WINDOWS_874: Encoding = Encoding(18);
/// windows-1250, a legacy single-byte encoding.
pub const WINDOWS_1250: Encoding = Encoding(19);
/// windows-1251, a legacy single-byte encoding.
pub const WINDOWS_1251: Encoding = Encoding(20);
/// windows-1252, a legacy single-byte encoding. Its labels include
/// `us-ascii`, `iso-8859-1` and `latin1`.
pub const WINDOWS_1252: Encoding = Encoding(21);
/// windows-1253, a legacy single-byte encoding.
pub const WINDOWS_1253: Encoding = Encoding(22);
/// windows-1254, a legacy single-byte encoding.
pub const WINDOWS_1254: Encoding = Encoding(23);
/// windows-1255, a legacy single-byte encoding.
pub const WINDOWS_1255: Encoding = Encoding(24);
/// windows-1256, a legacy single-byte encoding.
pub const WINDOWS_1256: Encoding = Encoding(25);
/// windows-1257, a legacy single-byte encoding.
pub const WINDOWS_1257: Encoding = Encoding(26);
/// windows-1258, a legacy single-byte encoding.
pub const WINDOWS_1258: Encoding = Encoding(27);
/// x-mac-cyrillic, a legacy single-byte encoding.
pub const X_MAC_CYRILLIC: Encoding = Encoding(28);
/// GBK, a legacy Chinese encoding; recognised, not yet decoded.
pub const GBK: Encoding = Encoding(29);
/// gb18030, a legacy Chinese encoding; recognised, not yet decoded.
pub const GB18030: Encoding = Encoding(30);
/// Big5, a legacy Chinese encoding; recognised, not yet decoded.
pub const BIG5: Encoding = Encoding(31);
/// EUC-JP, a legacy Japanese encoding; recognised, not yet decoded.
pub const EUC_JP: Encoding = Encoding(32);
/// ISO-2022-JP, a legacy Japanese encoding; recognised, not yet decoded.
pub const ISO_2022_JP: Encoding = Encoding(33);
/// Shift_JIS, a legacy Japanese encoding; recognised, not yet decoded.
pub const SHIFT_JIS: Encoding = Encoding(34);
/// EUC-KR, a legacy Korean encoding; recognised, not yet decoded.
pub const EUC_KR: Encoding = Encoding(35);
/// replacement: the encoding of labels such as `iso-2022-kr` whose
/// encodings are unsafe to decode; any input decodes to one U+FFFD.
pub const REPLACEMENT: Encoding = Encoding(36);
/// UTF-16BE.
pub const UTF_16BE: Encoding = Encoding(37);
/// UTF-16LE, which the label `utf-16` names, for deployed content.
pub const UTF_16LE: Encoding = Encoding(38);
/// x-user-defined: bytes 0x80 to 0xFF map to U+F780 to U+F7FF.
pub const X_USER_DEFINED: Encoding = Encoding(39);

/// Each encoding's name, as the standard spells it, in its order.
const NAMES: [&str; 40] = [
    "UTF-8",
    "IBM866",
    "ISO-8859-2",
    "ISO-8859-3",
    "ISO-8859-4",
    "ISO-8859-5",
    "ISO-8859-6",
    "ISO-8859-7",
    "ISO-8859-8",
    "ISO-8859-8-I",
    "ISO-8859-10",
    "ISO-8859-13",
    "ISO-8859-14",
    "ISO-8859-15",
    "ISO-8859-16",
    "KOI8-R",
    "KOI8-U",
    "macintosh",
    "windows-874",
    "windows-1250",
    "windows-1251",
    "windows-1252",
    "windows-1253",
    "windows-1254",
    "windows-1255",
    "windows-1256",
    "windows-1257",
    "windows-1258",
    "x-mac-cyrillic",
    "GBK",
    "gb18030",
    "Big5",
    "EUC-JP",
    "ISO-2022-JP",
    "Shift_JIS",
    "EUC-KR",
    "replacement",
    "UTF-16BE",
    "UTF-16LE",
    "x-user-defined",
];

impl Encoding {
    /// Every encoding, in the standard's order.
    pub fn all() -> impl Iterator<Item = Encoding> {
        (0..NAMES.len() as u8).map(Encoding)
    }

    /// The standard's *get an encoding*: the encoding `label` names,
    /// ignoring leading and trailing ASCII whitespace and ASCII case,
    /// or `None` when it names none (Encoding §4.2).
    pub fn for_label(label: &str) -> Option<Encoding> {
        Encoding::for_label_bytes(label.as_bytes())
    }

    /// [`Encoding::for_label`] over bytes, for labels read out of a
    /// document before it is decoded (HTML's prescan, CSS's
    /// `@charset`). A byte outside ASCII matches no label.
    pub fn for_label_bytes(label: &[u8]) -> Option<Encoding> {
        let label = trim_ascii_whitespace(label);
        // Anything longer than every label cannot match, so it is refused
        // before it is copied.
        if label.is_empty() || label.len() > tables::LONGEST_LABEL {
            return None;
        }
        let mut lower = [0u8; tables::LONGEST_LABEL];
        for (to, from) in lower.iter_mut().zip(label) {
            *to = from.to_ascii_lowercase();
        }
        let lower = &lower[..label.len()];
        tables::LABELS
            .binary_search_by(|(candidate, _)| candidate.as_bytes().cmp(lower))
            .ok()
            .map(|at| tables::LABELS[at].1)
    }

    /// The encoding's name, as the standard spells it (`UTF-8`,
    /// `windows-1252`, `Shift_JIS`).
    pub fn name(self) -> &'static str {
        NAMES[self.0 as usize]
    }

    /// The standard's *get an output encoding*: UTF-8 for replacement
    /// and the UTF-16 encodings, which have no encoder, and the encoding
    /// itself otherwise (Encoding §4.3). URL parsing and form submission
    /// encode with it.
    pub fn output_encoding(self) -> Encoding {
        if self == REPLACEMENT || self == UTF_16BE || self == UTF_16LE {
            UTF_8
        } else {
            self
        }
    }

    /// Whether this is one of the 28 legacy single-byte encodings.
    pub fn is_single_byte(self) -> bool {
        (1..=28).contains(&self.0)
    }

    /// Whether this crate can decode the encoding yet: every encoding
    /// but the legacy CJK ones.
    pub fn can_decode(self) -> bool {
        !(GBK.0..=EUC_KR.0).contains(&self.0)
    }

    /// A decoder for the encoding with the given error mode, or `None`
    /// for an encoding this crate cannot decode yet (the legacy CJK
    /// ones).
    pub fn new_decoder(self, mode: DecoderMode) -> Option<Decoder> {
        Decoder::new(self, mode)
    }

    /// An encoder for the encoding, or `None` when it has none: the
    /// standard gives replacement and the UTF-16 encodings no encoder
    /// (use [`Encoding::output_encoding`] first), and this crate does
    /// not encode the legacy CJK encodings yet.
    pub fn new_encoder(self) -> Option<Encoder> {
        Encoder::new(self)
    }

    /// The table of a single-byte encoding: entry `n` is the code point
    /// byte `0x80 + n` decodes to, or 0 where the index has none.
    fn single_byte_table(self) -> Option<&'static [u16; 128]> {
        self.is_single_byte()
            .then(|| tables::SINGLE_BYTE[self.0 as usize - 1])
    }
}

impl fmt::Debug for Encoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl fmt::Display for Encoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Strips the ASCII whitespace Infra names (TAB, LF, FF, CR and SPACE)
/// from both ends.
fn trim_ascii_whitespace(bytes: &[u8]) -> &[u8] {
    let is_space = |b: &u8| matches!(b, b'\t' | b'\n' | b'\x0C' | b'\r' | b' ');
    let start = bytes
        .iter()
        .position(|b| !is_space(b))
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|b| !is_space(b))
        .map_or(start, |at| at + 1);
    &bytes[start..end]
}

/// The standard's *BOM sniff*: the encoding a byte order mark at the
/// start of `bytes` names, and the mark's length, or `None` (Encoding
/// §6.1). Only three marks exist: UTF-8's, UTF-16BE's and UTF-16LE's.
pub fn bom_sniff(bytes: &[u8]) -> Option<(Encoding, usize)> {
    match bytes {
        [0xEF, 0xBB, 0xBF, ..] => Some((UTF_8, 3)),
        [0xFE, 0xFF, ..] => Some((UTF_16BE, 2)),
        [0xFF, 0xFE, ..] => Some((UTF_16LE, 2)),
        _ => None,
    }
}

/// The standard's *decode*: `bytes` decoded with the encoding a byte
/// order mark names, or else with `fallback`, errors replaced by U+FFFD
/// (Encoding §6.1). Returns the text and the encoding that decoded it.
///
/// A fallback this crate cannot decode yet (a legacy CJK encoding) is
/// decoded as UTF-8, and UTF-8 is returned: a caller that cares compares
/// it with what it asked for.
pub fn decode(bytes: &[u8], fallback: Encoding) -> (String, Encoding) {
    let (encoding, bom) = match bom_sniff(bytes) {
        Some(found) => found,
        None if fallback.can_decode() => (fallback, 0),
        None => (UTF_8, 0),
    };
    let mut text = String::with_capacity(bytes.len());
    let mut decoder = Decoder::new(encoding, DecoderMode::Replacement)
        .expect("every encoding chosen above has a decoder");
    let _ = decoder.decode(&bytes[bom..], true, &mut text);
    (text, encoding)
}

/// The standard's *UTF-8 decode*: `bytes` as UTF-8, a leading UTF-8 byte
/// order mark dropped and errors replaced by U+FFFD (Encoding §6).
pub fn utf8_decode(bytes: &[u8]) -> String {
    utf8_decode_without_bom(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes))
}

/// The standard's *UTF-8 decode without BOM*: `bytes` as UTF-8, errors
/// replaced by U+FFFD; a byte order mark decodes to U+FEFF.
pub fn utf8_decode_without_bom(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    let mut decoder = Decoder::new(UTF_8, DecoderMode::Replacement).expect("UTF-8 decodes");
    let _ = decoder.decode(bytes, true, &mut text);
    text
}

/// The standard's *UTF-8 decode without BOM or fail*: `bytes` as UTF-8,
/// or `None` if they are not.
pub fn utf8_decode_without_bom_or_fail(bytes: &[u8]) -> Option<String> {
    std::str::from_utf8(bytes).ok().map(str::to_owned)
}

/// The standard's *encode*, its legacy hook for HTML forms: `text` in
/// `encoding`'s output encoding, a character it cannot represent
/// written as `&#N;` (Encoding §6.1). An encoding this crate cannot
/// encode yet (a legacy CJK one) is encoded as UTF-8, and UTF-8 is
/// returned with the bytes.
pub fn encode(text: &str, encoding: Encoding) -> (Vec<u8>, Encoding) {
    let output = encoding.output_encoding();
    let (mut encoder, used) = match Encoder::new(output) {
        Some(encoder) => (encoder, output),
        None => (Encoder::new(UTF_8).expect("UTF-8 encodes"), UTF_8),
    };
    let mut bytes = Vec::with_capacity(text.len());
    let _ = encoder.encode(text, EncoderMode::Html, &mut bytes);
    (bytes, used)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_ignore_case_and_surrounding_whitespace() {
        assert_eq!(Encoding::for_label("utf-8"), Some(UTF_8));
        assert_eq!(Encoding::for_label(" \tUTF8\n"), Some(UTF_8));
        assert_eq!(Encoding::for_label("Latin1"), Some(WINDOWS_1252));
        assert_eq!(Encoding::for_label("utf-16"), Some(UTF_16LE));
        assert_eq!(Encoding::for_label("ISO-2022-KR"), Some(REPLACEMENT));
        assert_eq!(Encoding::for_label("sjis"), Some(SHIFT_JIS));
    }

    #[test]
    fn near_misses_are_no_label() {
        for label in ["", " ", "utf 8", "utf-8;", "utf-8\u{a0}", "\u{17f}jis", "x"] {
            assert_eq!(Encoding::for_label(label), None, "{label:?}");
        }
        // Longer than every label: refused without being copied.
        assert_eq!(Encoding::for_label(&"a".repeat(1000)), None);
        // Non-ASCII bytes match nothing, even where lowercasing would.
        assert_eq!(Encoding::for_label_bytes(b"utf\xC0-8"), None);
    }

    #[test]
    fn output_encodings() {
        assert_eq!(UTF_16LE.output_encoding(), UTF_8);
        assert_eq!(UTF_16BE.output_encoding(), UTF_8);
        assert_eq!(REPLACEMENT.output_encoding(), UTF_8);
        assert_eq!(WINDOWS_1252.output_encoding(), WINDOWS_1252);
        assert_eq!(SHIFT_JIS.output_encoding(), SHIFT_JIS);
    }

    #[test]
    fn names_and_counts() {
        assert_eq!(Encoding::all().count(), 40);
        assert_eq!(Encoding::all().filter(|e| e.is_single_byte()).count(), 28);
        assert_eq!(Encoding::all().filter(|e| !e.can_decode()).count(), 7);
        assert_eq!(format!("{:?}", X_MAC_CYRILLIC), "x-mac-cyrillic");
        assert_eq!(
            ISO_8859_8_I.single_byte_table(),
            ISO_8859_8.single_byte_table()
        );
    }

    #[test]
    fn decode_prefers_a_byte_order_mark() {
        assert_eq!(
            decode(b"\xEF\xBB\xBFa\xE9", WINDOWS_1252),
            ("a\u{FFFD}".into(), UTF_8)
        );
        assert_eq!(decode(b"\xFE\xFF\x00a", UTF_8), ("a".into(), UTF_16BE));
        assert_eq!(decode(b"\xFF\xFEa\x00", UTF_8), ("a".into(), UTF_16LE));
        assert_eq!(
            decode(b"a\xE9", WINDOWS_1252),
            ("a\u{E9}".into(), WINDOWS_1252)
        );
        // A fallback with no decoder yet decodes as UTF-8, and says so.
        assert_eq!(
            decode(b"a\x82\xA0", SHIFT_JIS),
            ("a\u{FFFD}\u{FFFD}".into(), UTF_8)
        );
    }

    #[test]
    fn utf8_hooks() {
        assert_eq!(utf8_decode(b"\xEF\xBB\xBFa"), "a");
        assert_eq!(utf8_decode_without_bom(b"\xEF\xBB\xBFa"), "\u{FEFF}a");
        assert_eq!(utf8_decode_without_bom_or_fail(b"a\xFF"), None);
        assert_eq!(
            utf8_decode_without_bom_or_fail(b"\xC3\xA9").as_deref(),
            Some("\u{E9}")
        );
    }

    #[test]
    fn encode_writes_what_it_cannot_represent_as_a_reference() {
        assert_eq!(
            encode("a\u{E9}\u{2603}", WINDOWS_1252),
            (b"a\xE9&#9731;".to_vec(), WINDOWS_1252)
        );
        assert_eq!(
            encode("\u{2603}", UTF_16LE),
            ("\u{2603}".as_bytes().to_vec(), UTF_8)
        );
        assert_eq!(
            encode("\u{E9}", EUC_KR),
            ("\u{E9}".as_bytes().to_vec(), UTF_8)
        );
    }
}
