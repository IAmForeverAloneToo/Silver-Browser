//! The crate against the Encoding Standard's own data in `data/`: the
//! label table, the single-byte indexes, and the generated `tables.rs`
//! they must match. Random inputs come from a fixed-seed generator, so
//! a failure replays.

use std::path::{Path, PathBuf};

use encoding::{DecodeError, DecoderMode, EncoderMode, Encoding, Unmappable};

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("data")
}

/// The standard's encodings, by group, from `encodings.json`: each
/// encoding's name and labels, in the standard's order.
fn standard_encodings() -> Vec<(String, String, Vec<String>)> {
    let json = std::fs::read_to_string(data_dir().join("encodings.json")).expect("encodings.json");
    let groups: serde_json::Value = serde_json::from_str(&json).expect("encodings.json is JSON");
    let mut out = Vec::new();
    for group in groups.as_array().expect("an array of groups") {
        let heading = group["heading"].as_str().expect("a heading").to_string();
        for encoding in group["encodings"].as_array().expect("encodings") {
            let name = encoding["name"].as_str().expect("a name").to_string();
            let labels = encoding["labels"]
                .as_array()
                .expect("labels")
                .iter()
                .map(|label| label.as_str().expect("a label").to_string())
                .collect();
            out.push((heading.clone(), name, labels));
        }
    }
    out
}

/// The single-byte encodings' names, in the standard's order.
fn single_byte_names() -> Vec<String> {
    standard_encodings()
        .into_iter()
        .filter(|(heading, _, _)| heading == "Legacy single-byte encodings")
        .map(|(_, name, _)| name)
        .collect()
}

/// The index file a single-byte encoding uses: its own, except that
/// ISO-8859-8-I shares ISO-8859-8's (Encoding §9).
fn index_file(name: &str) -> String {
    let name = if name == "ISO-8859-8-I" {
        "ISO-8859-8"
    } else {
        name
    };
    format!("index-{}.txt", name.to_ascii_lowercase())
}

/// An index file read as the standard defines the format (Encoding
/// §5): one pointer and one code point per line, `#` starting a
/// comment.
fn read_index(file: &str) -> Vec<(usize, u32)> {
    let text =
        std::fs::read_to_string(data_dir().join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
    text.lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|line| {
            let mut fields = line.split_whitespace();
            let pointer = fields
                .next()
                .and_then(|p| p.parse().ok())
                .expect("a pointer");
            let code_point = fields
                .next()
                .and_then(|c| c.strip_prefix("0x"))
                .and_then(|c| u32::from_str_radix(c, 16).ok())
                .expect("a code point");
            (pointer, code_point)
        })
        .collect()
}

/// The name of the crate's constant for an encoding: the standard's
/// name, uppercase, with `-` as `_`.
fn constant(name: &str) -> String {
    name.to_ascii_uppercase().replace('-', "_")
}

/// `tables.rs` as the data says it should be.
fn generate() -> String {
    let mut out = String::new();
    out.push_str(
        "//! Generated from `data/encodings.json` and the single-byte index files\n\
         //! in `data/` by `tests/standard.rs`; see `data/README.md`. Do not edit.\n\
         //!\n\
         //! Portions of the WHATWG Encoding Standard, under BSD-3-Clause: see\n\
         //! `LICENSE-WHATWG`.\n\n",
    );

    let mut labels: Vec<(String, String)> = standard_encodings()
        .into_iter()
        .flat_map(|(_, name, labels)| {
            labels
                .into_iter()
                .map(move |label| (label, constant(&name)))
        })
        .collect();
    labels.sort();
    out.push_str("/// Every label, sorted, with the encoding it names (Encoding §4.2).\n");
    out.push_str(&format!(
        "pub(crate) static LABELS: [(&str, crate::Encoding); {}] = [\n",
        labels.len()
    ));
    for (label, encoding) in &labels {
        out.push_str(&format!("    ({label:?}, crate::{encoding}),\n"));
    }
    out.push_str("];\n\n");
    let longest = labels
        .iter()
        .map(|(label, _)| label.len())
        .max()
        .expect("labels");
    out.push_str("/// The length of the longest label.\n");
    out.push_str(&format!(
        "pub(crate) const LONGEST_LABEL: usize = {longest};\n\n"
    ));

    let names = single_byte_names();
    out.push_str(
        "/// The single-byte encodings' indexes, in the standard's order: entry\n\
         /// `n` of a table is the code point for byte `0x80 + n`, 0 where the\n\
         /// index has none. ISO-8859-8-I shares ISO-8859-8's.\n",
    );
    out.push_str(&format!(
        "pub(crate) static SINGLE_BYTE: [&[u16; 128]; {}] = [\n",
        names.len()
    ));
    for name in &names {
        let file = index_file(name);
        let table = file.trim_start_matches("index-").trim_end_matches(".txt");
        out.push_str(&format!("    &INDEX_{},\n", constant(table)));
    }
    out.push_str("];\n");

    for name in names.iter().filter(|name| *name != "ISO-8859-8-I") {
        let mut table = [0u32; 128];
        for (pointer, code_point) in read_index(&index_file(name)) {
            assert!(
                pointer < 128 && code_point > 0 && code_point <= 0xFFFF,
                "{name}: {pointer}"
            );
            table[pointer] = code_point;
        }
        out.push_str(&format!(
            "\nstatic INDEX_{}: [u16; 128] = [\n",
            constant(name)
        ));
        for row in table.chunks(8) {
            let cells: Vec<String> = row.iter().map(|c| format!("0x{c:04X}")).collect();
            out.push_str(&format!("    {},\n", cells.join(", ")));
        }
        out.push_str("];\n");
    }
    out
}

#[test]
fn tables_match_the_standard() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tables.rs");
    let generated = generate();
    if std::env::var_os("ENCODING_REGENERATE").is_some() {
        std::fs::write(&path, &generated).expect("tables.rs written");
    }
    let current = std::fs::read_to_string(&path).expect("tables.rs");
    assert!(
        current == generated,
        "src/tables.rs does not match data/; see data/README.md to regenerate it"
    );
}

#[test]
fn the_crate_has_the_standards_encodings_in_its_order() {
    let names: Vec<String> = standard_encodings()
        .into_iter()
        .map(|(_, name, _)| name)
        .collect();
    let ours: Vec<&str> = Encoding::all().map(Encoding::name).collect();
    assert_eq!(ours, names);
    let labels: usize = standard_encodings()
        .iter()
        .map(|(_, _, labels)| labels.len())
        .sum();
    assert_eq!(labels, 228);
}

#[test]
fn every_label_names_its_encoding() {
    for (_, name, labels) in standard_encodings() {
        for label in labels {
            for written in [
                label.clone(),
                label.to_ascii_uppercase(),
                format!("\t {label}\r\n\x0C"),
            ] {
                let found = Encoding::for_label(&written);
                assert_eq!(
                    found.map(Encoding::name),
                    Some(name.as_str()),
                    "{written:?}"
                );
            }
            // Anything around the label but ASCII whitespace spoils it.
            assert_eq!(
                Encoding::for_label(&format!("{label}\u{A0}")),
                None,
                "{label:?}"
            );
            assert_eq!(Encoding::for_label(&format!("x{label}")), None, "{label:?}");
        }
    }
}

/// The single-byte encodings with the indexes they decode by, read from
/// the files independently of `tables.rs`.
fn single_byte_indexes() -> Vec<(Encoding, Vec<Option<char>>)> {
    single_byte_names()
        .into_iter()
        .map(|name| {
            let encoding =
                Encoding::for_label(&name).expect("a single-byte encoding's name is a label");
            let mut index = vec![None; 128];
            for (pointer, code_point) in read_index(&index_file(&name)) {
                index[pointer] = char::from_u32(code_point);
            }
            (encoding, index)
        })
        .collect()
}

fn decode_whole(
    encoding: Encoding,
    mode: DecoderMode,
    bytes: &[u8],
) -> Result<String, DecodeError> {
    let mut out = String::new();
    let mut decoder = encoding.new_decoder(mode).expect("a decoder");
    decoder.decode(bytes, true, &mut out).map(|()| out)
}

#[test]
fn single_byte_decoders_follow_their_indexes() {
    let indexes = single_byte_indexes();
    assert_eq!(indexes.len(), 28);
    for (encoding, index) in indexes {
        for byte in 0..=255u8 {
            let expected = if byte < 0x80 {
                Some(byte as char)
            } else {
                index[usize::from(byte - 0x80)]
            };
            let lossy = decode_whole(encoding, DecoderMode::Replacement, &[byte]).unwrap();
            assert_eq!(
                lossy,
                expected.unwrap_or('\u{FFFD}').to_string(),
                "{encoding:?} {byte:#04X}"
            );
            let fatal = decode_whole(encoding, DecoderMode::Fatal, &[byte]);
            assert_eq!(
                fatal.is_err(),
                expected.is_none(),
                "{encoding:?} {byte:#04X}"
            );
        }
    }
}

#[test]
fn single_byte_encoders_invert_their_indexes() {
    for (encoding, index) in single_byte_indexes() {
        let mut encoder = encoding
            .new_encoder()
            .expect("single-byte encodings encode");
        for (pointer, c) in index.iter().enumerate() {
            let Some(c) = *c else { continue };
            // The index pointer is the first that holds the code point.
            let first = index.iter().position(|entry| *entry == Some(c)).unwrap();
            assert_eq!(
                first, pointer,
                "{encoding:?}: indexes here hold each code point once"
            );
            let mut out = Vec::new();
            encoder.encode_char(c, &mut out).unwrap();
            assert_eq!(out, [0x80 + pointer as u8], "{encoding:?} {c:?}");
        }
        for c in ['\u{2603}', '\u{1F600}', '\u{FFFD}'] {
            if !index.contains(&Some(c)) {
                let mut out = Vec::new();
                assert_eq!(
                    encoder.encode_char(c, &mut out),
                    Err(Unmappable(c)),
                    "{encoding:?}"
                );
                assert!(out.is_empty());
            }
        }
        let mut out = Vec::new();
        encoder
            .encode("plain ASCII", EncoderMode::Fatal, &mut out)
            .unwrap();
        assert_eq!(out, b"plain ASCII");
    }
}

/// xorshift64: a sequence a failing seed replays.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    /// Bytes that are mostly almost UTF-8 or UTF-16, so the decoders'
    /// error paths are exercised rather than only their ASCII ones.
    fn bytes(&mut self, max_len: usize) -> Vec<u8> {
        const INTERESTING: [u8; 16] = [
            0x00, 0x41, 0x7F, 0x80, 0x8F, 0x90, 0x9F, 0xA0, 0xBF, 0xC2, 0xDF, 0xE0, 0xED, 0xF0,
            0xF4, 0xFF,
        ];
        const SURROGATES: [u8; 4] = [0xD8, 0xDB, 0xDC, 0xDF];
        let len = self.below(max_len + 1);
        (0..len)
            .map(|_| match self.below(4) {
                0 => INTERESTING[self.below(INTERESTING.len())],
                1 => SURROGATES[self.below(SURROGATES.len())],
                _ => self.next() as u8,
            })
            .collect()
    }
}

#[test]
fn utf8_decodes_as_std_does() {
    // std's lossy decoding replaces maximal subparts, as Unicode's best
    // practice and so the standard's UTF-8 decoder do: an oracle written
    // apart from this crate.
    let mut rng = Rng(0x5EED_0001);
    for _ in 0..20_000 {
        let bytes = rng.bytes(24);
        let ours = decode_whole(encoding::UTF_8, DecoderMode::Replacement, &bytes).unwrap();
        assert_eq!(ours, String::from_utf8_lossy(&bytes), "{bytes:02X?}");
        let fatal = decode_whole(encoding::UTF_8, DecoderMode::Fatal, &bytes);
        assert_eq!(
            fatal.ok().as_deref(),
            std::str::from_utf8(&bytes).ok(),
            "{bytes:02X?}"
        );
    }
}

#[test]
fn utf16_decodes_as_std_does() {
    // For an even number of bytes the standard's UTF-16 decoder replaces
    // each unpaired surrogate as std's does.
    let mut rng = Rng(0x5EED_0002);
    for _ in 0..20_000 {
        let mut bytes = rng.bytes(24);
        bytes.truncate(bytes.len() & !1);
        for (encoding, big_endian) in [(encoding::UTF_16LE, false), (encoding::UTF_16BE, true)] {
            let units: Vec<u16> = bytes
                .chunks(2)
                .map(|pair| {
                    let pair = [pair[0], pair[1]];
                    if big_endian {
                        u16::from_be_bytes(pair)
                    } else {
                        u16::from_le_bytes(pair)
                    }
                })
                .collect();
            let ours = decode_whole(encoding, DecoderMode::Replacement, &bytes).unwrap();
            assert_eq!(
                ours,
                String::from_utf16_lossy(&units),
                "{encoding:?} {bytes:02X?}"
            );
        }
    }
}

/// Decodes `bytes` cut at `cuts`, each piece a separate call.
fn decode_in_pieces(
    encoding: Encoding,
    mode: DecoderMode,
    bytes: &[u8],
    cuts: &[usize],
) -> (String, Result<(), DecodeError>) {
    let mut out = String::new();
    let mut decoder = encoding.new_decoder(mode).expect("a decoder");
    let mut start = 0;
    for &cut in cuts {
        if let Err(e) = decoder.decode(&bytes[start..cut], false, &mut out) {
            return (out, Err(e));
        }
        start = cut;
    }
    let result = decoder.decode(&bytes[start..], true, &mut out);
    (out, result)
}

#[test]
fn every_decoder_decodes_pieces_as_it_decodes_whole() {
    let mut rng = Rng(0x5EED_0003);
    let decodable: Vec<Encoding> = Encoding::all().filter(|e| e.can_decode()).collect();
    assert_eq!(decodable.len(), 33);
    for _ in 0..4_000 {
        let bytes = rng.bytes(32);
        let mut cuts: Vec<usize> = (0..rng.below(6))
            .map(|_| rng.below(bytes.len() + 1))
            .collect();
        cuts.sort_unstable();
        for &encoding in &decodable {
            for mode in [DecoderMode::Replacement, DecoderMode::Fatal] {
                let mut whole = String::new();
                let mut decoder = encoding.new_decoder(mode).unwrap();
                let whole_result = decoder.decode(&bytes, true, &mut whole);
                let (pieces, pieces_result) = decode_in_pieces(encoding, mode, &bytes, &cuts);
                assert_eq!(
                    pieces_result, whole_result,
                    "{encoding:?} {mode:?} {bytes:02X?} {cuts:?}"
                );
                assert_eq!(pieces, whole, "{encoding:?} {mode:?} {bytes:02X?} {cuts:?}");
                if mode == DecoderMode::Fatal && whole_result.is_ok() {
                    let lossy = decode_whole(encoding, DecoderMode::Replacement, &bytes).unwrap();
                    assert_eq!(whole, lossy, "{encoding:?} {bytes:02X?}");
                }
            }
        }
    }
}
