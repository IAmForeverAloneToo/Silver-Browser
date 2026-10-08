//! The MIME type sniffing algorithm (MIME Sniffing §5 to §7): a
//! resource's computed type, from the type it was supplied with and its
//! first 1445 bytes. This is the algorithm a browsing context uses
//! (§8.1); a stylesheet takes its supplied type as it is (§8.5).

use super::MimeType;
use crate::headers::HeaderList;

/// How many bytes of a resource sniffing reads: its resource header
/// (§5.2).
pub const RESOURCE_HEADER_LENGTH: usize = 1445;

/// The check-for-apache-bug flag (§5.1): whether the last `Content-Type`
/// field is exactly one of the four values old Apache servers sent for
/// files of a type they did not know.
pub fn check_for_apache_bug(headers: &HeaderList) -> bool {
    headers.last(b"Content-Type").is_some_and(|value| {
        [
            &b"text/plain"[..],
            b"text/plain; charset=ISO-8859-1",
            b"text/plain; charset=iso-8859-1",
            b"text/plain; charset=UTF-8",
        ]
        .contains(&value)
    })
}

/// *The MIME type sniffing algorithm* (§7). `supplied` is the supplied
/// MIME type, if any; `body` is the resource, of which the first
/// [`RESOURCE_HEADER_LENGTH`] bytes are read.
pub fn computed_type(
    supplied: Option<&MimeType>,
    check_for_apache_bug: bool,
    no_sniff: bool,
    body: &[u8],
) -> MimeType {
    let header = resource_header(body);
    let supplied = match supplied {
        Some(supplied) if supplied.is_xml() || supplied.is_html() => return supplied.clone(),
        Some(supplied)
            if !matches!(
                supplied.essence(),
                "unknown/unknown" | "application/unknown" | "*/*"
            ) =>
        {
            supplied
        }
        _ => return identify_unknown(header, !no_sniff),
    };
    if no_sniff {
        return supplied.clone();
    }
    if check_for_apache_bug {
        return text_or_binary(header);
    }
    // Steps 5 to 8: an image type the engine decodes is checked against
    // the image signatures (M23); it decodes no audio or video.
    if SUPPORTED_IMAGES.contains(&supplied.essence())
        && let Some(row) = IMAGE.iter().find(|row| row.matches(header))
    {
        return MimeType::known(row.mime_type);
    }
    supplied.clone()
}

/// *The rules for identifying an unknown MIME type* (§7.1), on the
/// resource header of `body`. Without `sniff_scriptable`, the result is
/// never a scriptable type.
pub fn identify_unknown(body: &[u8], sniff_scriptable: bool) -> MimeType {
    let header = resource_header(body);
    if sniff_scriptable && let Some(row) = SCRIPTABLE.iter().find(|row| row.matches(header)) {
        return MimeType::known(row.mime_type);
    }
    let matched = OTHER
        .iter()
        .find(|row| row.matches(header))
        .map(|row| row.mime_type)
        .or_else(|| image_type(header))
        .or_else(|| audio_or_video_type(header))
        .or_else(|| archive_type(header));
    if let Some(mime_type) = matched {
        return MimeType::known(mime_type);
    }
    if header.iter().any(|&b| is_binary_data_byte(b)) {
        MimeType::known("application/octet-stream")
    } else {
        MimeType::known("text/plain")
    }
}

/// *The rules for distinguishing if a resource is text or binary*
/// (§7.2), on the resource header of `body`: `text/plain` for a BOM or
/// for no binary data byte, and `application/octet-stream` otherwise.
/// Never a scriptable type.
pub fn text_or_binary(body: &[u8]) -> MimeType {
    let header = resource_header(body);
    let has_bom = header.starts_with(&[0xFE, 0xFF])
        || header.starts_with(&[0xFF, 0xFE])
        || header.starts_with(&[0xEF, 0xBB, 0xBF]);
    if has_bom || !header.iter().any(|&b| is_binary_data_byte(b)) {
        MimeType::known("text/plain")
    } else {
        MimeType::known("application/octet-stream")
    }
}

/// The resource header (§5.2): the first [`RESOURCE_HEADER_LENGTH`]
/// bytes, or all of them when there are fewer. The whole body is in
/// hand, so no read stops early.
fn resource_header(body: &[u8]) -> &[u8] {
    &body[..body.len().min(RESOURCE_HEADER_LENGTH)]
}

/// *The image type pattern matching algorithm* (§6.1).
pub fn image_type(input: &[u8]) -> Option<&'static str> {
    IMAGE
        .iter()
        .find(|row| row.matches(input))
        .map(|row| row.mime_type)
}

/// *The audio or video type pattern matching algorithm* (§6.2).
pub fn audio_or_video_type(input: &[u8]) -> Option<&'static str> {
    if let Some(row) = AUDIO_OR_VIDEO.iter().find(|row| row.matches(input)) {
        Some(row.mime_type)
    } else if matches_mp4(input) {
        Some("video/mp4")
    } else if matches_webm(input) {
        Some("video/webm")
    } else if matches_mp3_without_id3(input) {
        Some("audio/mpeg")
    } else {
        None
    }
}

/// *The archive type pattern matching algorithm* (§6.4).
pub fn archive_type(input: &[u8]) -> Option<&'static str> {
    ARCHIVE
        .iter()
        .find(|row| row.matches(input))
        .map(|row| row.mime_type)
}

/// A binary data byte (§3): 0x00 to 0x08, 0x0B, 0x0E to 0x1A, and 0x1C
/// to 0x1F.
fn is_binary_data_byte(b: u8) -> bool {
    matches!(b, 0x00..=0x08 | 0x0B | 0x0E..=0x1A | 0x1C..=0x1F)
}

/// A whitespace byte (§3): HT, LF, FF, CR and SP.
fn is_whitespace_byte(b: u8) -> bool {
    matches!(b, 0x09 | 0x0A | 0x0C | 0x0D | 0x20)
}

/// A row of one of the standard's pattern tables.
struct Row {
    pattern: &'static [u8],
    mask: Mask,
    /// Whether leading whitespace bytes are ignored; no row ignores any
    /// other bytes.
    skip_whitespace: bool,
    /// Whether the pattern ends in a tag-terminating byte, SP or `>`,
    /// which the tables write as `TT` with the mask `FF`.
    tag_terminated: bool,
    mime_type: &'static str,
}

/// A row's pattern mask.
#[derive(Clone, Copy)]
enum Mask {
    /// `FF` for every byte: the bytes as they are.
    Exact,
    /// The mask the table gives.
    Given(&'static [u8]),
    /// `DF` for each letter, which the pattern has in upper case, so that
    /// it matches in either case, and `FF` for every other byte. Every
    /// HTML row of §7.1 has this mask.
    LettersInAnyCase,
}

impl Row {
    /// The mask byte for the pattern's byte at `p`.
    fn mask_at(&self, p: usize) -> u8 {
        match self.mask {
            Mask::Exact => 0xFF,
            Mask::Given(mask) => mask[p],
            Mask::LettersInAnyCase if self.pattern[p].is_ascii_uppercase() => 0xDF,
            Mask::LettersInAnyCase => 0xFF,
        }
    }

    /// *The pattern matching algorithm* (§6) for this row.
    fn matches(&self, input: &[u8]) -> bool {
        if let Mask::Given(mask) = self.mask {
            debug_assert_eq!(self.pattern.len(), mask.len());
        }
        let length = self.pattern.len() + usize::from(self.tag_terminated);
        if input.len() < length {
            return false;
        }
        let mut s = 0;
        if self.skip_whitespace {
            while s < input.len() && is_whitespace_byte(input[s]) {
                s += 1;
            }
        }
        // The standard reads on past the end when the skipped bytes
        // leave fewer than the pattern's; that is no match.
        let Some(rest) = input.get(s..s + length) else {
            return false;
        };
        let matched = self
            .pattern
            .iter()
            .enumerate()
            .all(|(p, &pattern)| rest[p] & self.mask_at(p) == pattern);
        matched && (!self.tag_terminated || matches!(rest[length - 1], b' ' | b'>'))
    }
}

/// A row matched as the bytes are: every byte significant, nothing
/// skipped.
const fn exact(pattern: &'static [u8], mime_type: &'static str) -> Row {
    Row {
        pattern,
        mask: Mask::Exact,
        skip_whitespace: false,
        tag_terminated: false,
        mime_type,
    }
}

/// A row with the mask the table gives.
const fn masked(pattern: &'static [u8], mask: &'static [u8], mime_type: &'static str) -> Row {
    Row {
        mask: Mask::Given(mask),
        ..exact(pattern, mime_type)
    }
}

/// An HTML row of §7.1: leading whitespace skipped, letters matched in
/// any case, and a tag-terminating byte after. `upper` is the pattern
/// as the table writes it, its letters upper case.
const fn html_tag(upper: &'static [u8]) -> Row {
    Row {
        pattern: upper,
        mask: Mask::LettersInAnyCase,
        skip_whitespace: true,
        tag_terminated: true,
        mime_type: "text/html",
    }
}

/// §7.1 step 1, the scriptable types, in the table's order.
static SCRIPTABLE: [Row; 19] = [
    html_tag(b"<!DOCTYPE HTML"),
    html_tag(b"<HTML"),
    html_tag(b"<HEAD"),
    html_tag(b"<SCRIPT"),
    html_tag(b"<IFRAME"),
    html_tag(b"<H1"),
    html_tag(b"<DIV"),
    html_tag(b"<FONT"),
    html_tag(b"<TABLE"),
    html_tag(b"<A"),
    html_tag(b"<STYLE"),
    html_tag(b"<TITLE"),
    html_tag(b"<B"),
    html_tag(b"<BODY"),
    html_tag(b"<BR"),
    html_tag(b"<P"),
    html_tag(b"<!--"),
    Row {
        skip_whitespace: true,
        ..exact(b"<?xml", "text/xml")
    },
    exact(b"%PDF-", "application/pdf"),
];

/// §7.1 step 2: PostScript, and text with a byte order mark.
static OTHER: [Row; 4] = [
    exact(b"%!PS-Adobe-", "application/postscript"),
    masked(
        &[0xFE, 0xFF, 0x00, 0x00],
        &[0xFF, 0xFF, 0x00, 0x00],
        "text/plain",
    ),
    masked(
        &[0xFF, 0xFE, 0x00, 0x00],
        &[0xFF, 0xFF, 0x00, 0x00],
        "text/plain",
    ),
    masked(
        &[0xEF, 0xBB, 0xBF, 0x00],
        &[0xFF, 0xFF, 0xFF, 0x00],
        "text/plain",
    ),
];

/// The mask of a RIFF-style row: the first four bytes, any four, then
/// the rest exactly.
const RIFF_MASK_12: &[u8] = &[
    0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF,
];

/// The image types the engine decodes (M23).
const SUPPORTED_IMAGES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/// §6.1's table.
static IMAGE: [Row; 8] = [
    exact(&[0x00, 0x00, 0x01, 0x00], "image/x-icon"),
    exact(&[0x00, 0x00, 0x02, 0x00], "image/x-icon"),
    exact(b"BM", "image/bmp"),
    exact(b"GIF87a", "image/gif"),
    exact(b"GIF89a", "image/gif"),
    masked(
        b"RIFF\0\0\0\0WEBPVP",
        &[
            0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        ],
        "image/webp",
    ),
    exact(
        &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
        "image/png",
    ),
    exact(&[0xFF, 0xD8, 0xFF], "image/jpeg"),
];

/// §6.2's table; the MP4, WebM and MP3 signatures follow it.
static AUDIO_OR_VIDEO: [Row; 6] = [
    masked(b"FORM\0\0\0\0AIFF", RIFF_MASK_12, "audio/aiff"),
    exact(b"ID3", "audio/mpeg"),
    exact(b"OggS\0", "application/ogg"),
    exact(b"MThd\0\0\0\x06", "audio/midi"),
    masked(b"RIFF\0\0\0\0AVI ", RIFF_MASK_12, "video/avi"),
    masked(b"RIFF\0\0\0\0WAVE", RIFF_MASK_12, "audio/wave"),
];

/// §6.4's table.
static ARCHIVE: [Row; 3] = [
    exact(&[0x1F, 0x8B, 0x08], "application/x-gzip"),
    exact(b"PK\x03\x04", "application/zip"),
    exact(b"Rar!\x1A\x07\0", "application/x-rar-compressed"),
];

/// *Matches the signature for MP4* (§6.2.1): an `ftyp` box whose major
/// or a compatible brand starts with `mp4`.
fn matches_mp4(sequence: &[u8]) -> bool {
    let length = sequence.len();
    if length < 12 {
        return false;
    }
    let box_size = u32::from_be_bytes([sequence[0], sequence[1], sequence[2], sequence[3]]);
    let Ok(box_size) = usize::try_from(box_size) else {
        return false;
    };
    if length < box_size || !box_size.is_multiple_of(4) {
        return false;
    }
    if &sequence[4..8] != b"ftyp" {
        return false;
    }
    if &sequence[8..11] == b"mp4" {
        return true;
    }
    // Past the major brand's version.
    let mut bytes_read = 16;
    while bytes_read < box_size {
        // A multiple of four below a box size that is one, so four bytes
        // are there.
        if &sequence[bytes_read..bytes_read + 3] == b"mp4" {
            return true;
        }
        bytes_read += 4;
    }
    false
}

/// *Matches the signature for WebM* (§6.2.2): an EBML header whose
/// `DocType` element, within its first 38 bytes, says `webm`.
fn matches_webm(sequence: &[u8]) -> bool {
    let length = sequence.len();
    if length < 4 || sequence[..4] != [0x1A, 0x45, 0xDF, 0xA3] {
        return false;
    }
    let mut iter = 4;
    while iter < length && iter < 38 {
        if sequence[iter..].starts_with(&[0x42, 0x82]) {
            iter += 2;
            if iter >= length {
                return false;
            }
            let (number_size, size) = parse_vint(sequence, iter);
            iter += number_size;
            if iter >= length - 4 {
                return false;
            }
            if matches_padded_sequence(sequence, iter, size, b"webm") {
                return true;
            }
        }
        iter += 1;
    }
    false
}

/// *Parse a vint* at `iter`: EBML's variable-length integer, whose
/// leading zero bits give its length. Returns the length in bytes and the
/// value.
///
/// The standard starts reading at index 0, the first byte of the whole
/// sequence; it means the vint's own first byte, at `iter`, and that is
/// read here. A vint cut off by the end of the sequence keeps what was
/// read, as the standard's loop does.
fn parse_vint(sequence: &[u8], iter: usize) -> (usize, u64) {
    const MAX_VINT_LENGTH: usize = 8;
    let length = sequence.len();
    let mut mask: u8 = 128;
    let mut number_size = 1;
    let mut index = iter;
    while number_size < MAX_VINT_LENGTH && number_size < length {
        if sequence[index] & mask != 0 {
            break;
        }
        mask >>= 1;
        number_size += 1;
    }
    let mut parsed_number = u64::from(sequence[index] & !mask);
    index += 1;
    let mut bytes_remaining = number_size - 1;
    while bytes_remaining != 0 && index < length {
        parsed_number = parsed_number << 8 | u64::from(sequence[index]);
        index += 1;
        bytes_remaining -= 1;
    }
    (number_size, parsed_number)
}

/// *Matching a padded sequence*: the `size` bytes at `offset`, all within
/// the sequence, are `pattern` with zero or more 0x00 bytes before it.
/// The standard leaves the range's end unnamed at its one use; it is
/// where the element whose size the vint gave ends.
fn matches_padded_sequence(sequence: &[u8], offset: usize, size: u64, pattern: &[u8]) -> bool {
    let Some(data) = usize::try_from(size)
        .ok()
        .and_then(|size| sequence.get(offset..offset.checked_add(size)?))
    else {
        return false;
    };
    match data.len().checked_sub(pattern.len()) {
        Some(padding) => data[padding..] == *pattern && data[..padding].iter().all(|&b| b == 0),
        None => false,
    }
}

/// *Matches the signature for MP3 without ID3* (§6.2.3): an MPEG audio
/// layer III frame header, and another one frame size later.
///
/// Where the standard's text cannot mean what it says, this follows its
/// meaning: a frame sync is 0xFF *and* three set bits, not either; each
/// field is masked and then shifted, as its final-layer step writes with
/// parentheses; the next header must lie within the `length - s` bytes
/// left, where the text says `s - length`; and the second header must
/// have its four bytes. The rate tables and the frame size are as the
/// standard has them.
fn matches_mp3_without_id3(sequence: &[u8]) -> bool {
    let length = sequence.len();
    let mut s = 0;
    let Some(freq) = match_mp3_header(sequence, s) else {
        return false;
    };
    let frame = parse_mp3_frame(sequence, s);
    let skipped_bytes = mp3_frame_size(&frame, freq);
    if skipped_bytes < 4 || skipped_bytes > length - s {
        return false;
    }
    s += skipped_bytes;
    match_mp3_header(sequence, s).is_some()
}

/// The sample-rate table.
const SAMPLE_RATES: [u32; 3] = [44_100, 48_000, 32_000];
/// The mp3-rates table, indexed by bit-rate.
const MP3_RATES: [u32; 15] = [
    0, 32_000, 40_000, 48_000, 56_000, 64_000, 80_000, 96_000, 112_000, 128_000, 160_000, 192_000,
    224_000, 256_000, 320_000,
];
/// The mp2.5-rates table, indexed by bit-rate.
const MP25_RATES: [u32; 15] = [
    0, 8_000, 16_000, 24_000, 32_000, 40_000, 48_000, 56_000, 64_000, 80_000, 96_000, 112_000,
    128_000, 144_000, 160_000,
];

/// *Match an mp3 header* at `s`: on a match, the frequency its
/// sample-rate field names, which the frame size uses.
fn match_mp3_header(sequence: &[u8], s: usize) -> Option<u32> {
    let header = sequence.get(s..s.checked_add(4)?)?;
    if header[0] != 0xFF || header[1] & 0xE0 != 0xE0 {
        return None;
    }
    let layer = (header[1] & 0x06) >> 1;
    if layer == 0 {
        return None;
    }
    let bit_rate = (header[2] & 0xF0) >> 4;
    if bit_rate == 15 {
        return None;
    }
    let sample_rate = (header[2] & 0x0C) >> 2;
    if sample_rate == 3 {
        return None;
    }
    let freq = SAMPLE_RATES[usize::from(sample_rate)];
    let final_layer = 4 - layer;
    (final_layer == 3).then_some(freq)
}

/// What *parse an mp3 frame* reads.
struct Mp3Frame {
    version: u8,
    bitrate: u32,
    pad: bool,
}

/// *Parse an mp3 frame* at `s`, a header [`match_mp3_header`] accepted.
/// The standard also works out a sample rate that nothing reads; the
/// frame size uses the header's frequency.
fn parse_mp3_frame(sequence: &[u8], s: usize) -> Mp3Frame {
    let version = (sequence[s + 1] & 0x18) >> 3;
    let bitrate_index = usize::from((sequence[s + 2] & 0xF0) >> 4);
    let bitrate = if version & 0x01 != 0 {
        MP25_RATES[bitrate_index]
    } else {
        MP3_RATES[bitrate_index]
    };
    Mp3Frame {
        version,
        bitrate,
        pad: (sequence[s + 2] & 0x02) >> 1 != 0,
    }
}

/// *Compute an mp3 frame size*, in whole bytes.
fn mp3_frame_size(frame: &Mp3Frame, freq: u32) -> usize {
    let scale = if frame.version == 1 { 72 } else { 144 };
    let size = u64::from(frame.bitrate) * scale / u64::from(freq) + u64::from(frame.pad);
    usize::try_from(size).unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn computed(supplied: Option<&str>, apache: bool, no_sniff: bool, body: &[u8]) -> String {
        let supplied = supplied.map(|s| MimeType::parse(s).expect(s));
        computed_type(supplied.as_ref(), apache, no_sniff, body).to_string()
    }

    fn unknown(header: &[u8]) -> String {
        identify_unknown(header, true).to_string()
    }

    #[test]
    fn a_supplied_html_or_xml_type_is_trusted() {
        let pdf = b"%PDF-1.7 \x00\x01";
        assert_eq!(
            computed(Some("text/html;charset=utf-8"), false, false, pdf),
            "text/html;charset=utf-8"
        );
        assert_eq!(
            computed(Some("image/svg+xml"), true, false, pdf),
            "image/svg+xml"
        );
        assert_eq!(computed(Some("text/xml"), false, true, pdf), "text/xml");
    }

    #[test]
    fn a_missing_or_unknown_type_is_identified() {
        let page = b"\n <!doctype html>";
        for supplied in [
            None,
            Some("unknown/unknown"),
            Some("application/unknown"),
            Some("*/*"),
        ] {
            assert_eq!(
                computed(supplied, false, false, page),
                "text/html",
                "{supplied:?}"
            );
            // nosniff turns the scriptable types off.
            assert_eq!(
                computed(supplied, false, true, page),
                "text/plain",
                "{supplied:?}"
            );
        }
        assert_eq!(
            computed(None, false, true, b"%PDF-1.7\x00"),
            "application/octet-stream"
        );
        assert_eq!(computed(None, false, true, b"GIF89a"), "image/gif");
    }

    #[test]
    fn nosniff_keeps_the_supplied_type() {
        assert_eq!(
            computed(Some("text/plain"), true, true, b"\x00\x01"),
            "text/plain"
        );
        assert_eq!(computed(Some("x/y;a=b"), false, true, b"<html>"), "x/y;a=b");
    }

    #[test]
    fn the_apache_values_are_checked_for_text() {
        assert_eq!(
            computed(Some("text/plain"), true, false, b"\x00\x01"),
            "application/octet-stream"
        );
        assert_eq!(
            computed(Some("text/plain"), true, false, b"plain text\r\n"),
            "text/plain"
        );
        assert_eq!(
            computed(Some("text/plain"), true, false, b"\xFE\xFF\x00\x01"),
            "text/plain"
        );
        // Text never becomes a scriptable type this way.
        assert_eq!(
            computed(Some("text/plain"), true, false, b"<html>"),
            "text/plain"
        );
        assert_eq!(
            computed(Some("text/plain"), false, false, b"\x00\x01"),
            "text/plain"
        );
    }

    #[test]
    fn a_decoded_image_type_is_checked_and_other_types_stand() {
        // The engine decodes PNG, JPEG, GIF and WebP (M23).
        assert_eq!(
            computed(Some("image/png"), false, false, b"GIF89a"),
            "image/gif"
        );
        assert_eq!(
            computed(Some("image/png"), false, false, b"no signature"),
            "image/png"
        );
        assert_eq!(
            computed(Some("image/avif"), false, false, b"GIF89a"),
            "image/avif"
        );
        assert_eq!(
            computed(Some("application/json"), false, false, b"<html>"),
            "application/json"
        );
    }

    #[test]
    fn the_apache_flag_wants_one_of_four_exact_values_last() {
        let flag = |values: &[&[u8]]| {
            let mut headers = HeaderList::new();
            for value in values {
                headers.append(b"Content-Type", value);
            }
            check_for_apache_bug(&headers)
        };
        assert!(flag(&[b"text/plain"]));
        assert!(flag(&[b"text/plain; charset=ISO-8859-1"]));
        assert!(flag(&[b"text/plain; charset=iso-8859-1"]));
        assert!(flag(&[b"text/plain; charset=UTF-8"]));
        assert!(flag(&[b"text/html", b"text/plain"]));
        assert!(!flag(&[b"text/plain", b"text/html"]));
        assert!(!flag(&[b"text/plain; charset=utf-8"]));
        assert!(!flag(&[b"text/plain;charset=UTF-8"]));
        assert!(!flag(&[b"Text/plain"]));
        assert!(!flag(&[b"text/plain "]));
        assert!(!flag(&[]));
    }

    #[test]
    fn every_html_pattern_wants_a_tag_terminating_byte() {
        let tags = [
            "<!DOCTYPE HTML",
            "<HTML",
            "<HEAD",
            "<SCRIPT",
            "<IFRAME",
            "<H1",
            "<DIV",
            "<FONT",
            "<TABLE",
            "<A",
            "<STYLE",
            "<TITLE",
            "<B",
            "<BODY",
            "<BR",
            "<P",
            "<!--",
        ];
        assert_eq!(tags.len() + 2, SCRIPTABLE.len());
        for tag in tags {
            for end in [" ", ">"] {
                assert_eq!(
                    unknown(format!("{tag}{end}").as_bytes()),
                    "text/html",
                    "{tag}"
                );
                let lower = tag.to_ascii_lowercase();
                assert_eq!(
                    unknown(format!(" \t\n\x0C\r{lower}{end}").as_bytes()),
                    "text/html"
                );
            }
            for end in ["", "x", "\t", "/"] {
                assert_eq!(
                    unknown(format!("{tag}{end}").as_bytes()),
                    "text/plain",
                    "{tag}{end:?}"
                );
            }
            // The sniff-scriptable flag off.
            assert_eq!(
                identify_unknown(format!("{tag}>").as_bytes(), false).to_string(),
                "text/plain"
            );
        }
        // Letters only match in any case; "<H1" has a digit, "<!--" none.
        assert_eq!(unknown(b"<h1>"), "text/html");
        assert_eq!(unknown(b"<!DOCTYPE\x00HTML>"), "application/octet-stream");
        assert_eq!(unknown(b"<!doctype\x20html>"), "text/html");
        // A vertical tab is not a whitespace byte.
        assert_eq!(unknown(b"\x0B<html>"), "application/octet-stream");
    }

    #[test]
    fn the_html_masks_are_the_tables() {
        let mask =
            |row: &Row| -> Vec<u8> { (0..row.pattern.len()).map(|p| row.mask_at(p)).collect() };
        // As §7.1 writes them, without the TT's own FF.
        assert_eq!(
            mask(&SCRIPTABLE[0]),
            [
                0xFF, 0xFF, 0xDF, 0xDF, 0xDF, 0xDF, 0xDF, 0xDF, 0xDF, 0xFF, 0xDF, 0xDF, 0xDF, 0xDF
            ]
        );
        assert_eq!(mask(&SCRIPTABLE[5]), [0xFF, 0xDF, 0xFF]);
        assert_eq!(mask(&SCRIPTABLE[16]), [0xFF, 0xFF, 0xFF, 0xFF]);
        for row in &SCRIPTABLE[..17] {
            assert_eq!(row.pattern, row.pattern.to_ascii_uppercase());
        }
    }

    #[test]
    fn xml_and_pdf() {
        assert_eq!(unknown(b"  <?xml version"), "text/xml");
        assert_eq!(unknown(b"<?XML"), "text/plain");
        assert_eq!(unknown(b"%PDF-1.7"), "application/pdf");
        // Only the HTML and XML rows skip whitespace.
        assert_eq!(unknown(b" %PDF-1.7"), "text/plain");
        assert_eq!(
            identify_unknown(b"%PDF-1.7", false).to_string(),
            "text/plain"
        );
    }

    #[test]
    fn postscript_and_byte_order_marks() {
        assert_eq!(unknown(b"%!PS-Adobe-3.0"), "application/postscript");
        assert_eq!(
            identify_unknown(b"%!PS-Adobe-3.0", false).to_string(),
            "application/postscript"
        );
        assert_eq!(unknown(&[0xFE, 0xFF, 0x00, 0x01]), "text/plain");
        assert_eq!(unknown(&[0xFF, 0xFE, 0x01, 0x00]), "text/plain");
        assert_eq!(unknown(&[0xEF, 0xBB, 0xBF, 0x01]), "text/plain");
        // A BOM row wants four bytes; two alone are binary or not by
        // their bytes.
        assert_eq!(unknown(&[0xFE, 0xFF]), "text/plain");
        assert_eq!(unknown(&[0xFF, 0xFE, 0x00]), "application/octet-stream");
    }

    #[test]
    fn images() {
        let cases: [(&[u8], &str); 9] = [
            (&[0, 0, 1, 0], "image/x-icon"),
            (&[0, 0, 2, 0], "image/x-icon"),
            (b"BM", "image/bmp"),
            (b"GIF87a", "image/gif"),
            (b"GIF89a", "image/gif"),
            (b"RIFF\x01\x02\x03\x04WEBPVP8 ", "image/webp"),
            (b"\x89PNG\r\n\x1A\n", "image/png"),
            (&[0xFF, 0xD8, 0xFF], "image/jpeg"),
            (b"GIF89a and more", "image/gif"),
        ];
        for (input, expected) in cases {
            assert_eq!(image_type(input), Some(expected), "{input:?}");
            assert_eq!(unknown(input), expected, "{input:?}");
        }
        assert_eq!(image_type(b"GIF88a"), None);
        assert_eq!(image_type(b"RIFF\x01\x02\x03\x04WEBPXX"), None);
        assert_eq!(image_type(&[0xFF, 0xD8]), None);
        assert_eq!(image_type(b"bm"), None);
    }

    #[test]
    fn audio_and_video_tables() {
        let cases: [(&[u8], &str); 6] = [
            (b"FORM\x00\x00\x10\x00AIFF", "audio/aiff"),
            (b"ID3\x04", "audio/mpeg"),
            (b"OggS\x00\x02", "application/ogg"),
            (b"MThd\x00\x00\x00\x06\x00\x01", "audio/midi"),
            (b"RIFF\xFF\xFF\xFF\xFFAVI LIST", "video/avi"),
            (b"RIFF\x24\x08\x00\x00WAVEfmt ", "audio/wave"),
        ];
        for (input, expected) in cases {
            assert_eq!(audio_or_video_type(input), Some(expected), "{input:?}");
            assert_eq!(unknown(input), expected, "{input:?}");
        }
        assert_eq!(audio_or_video_type(b"OggS\x01"), None);
        assert_eq!(audio_or_video_type(b"MThd\x00\x00\x00\x07"), None);
    }

    fn ftyp(major: &[u8; 4], compatible: &[&[u8; 4]]) -> Vec<u8> {
        let size = 16 + 4 * compatible.len();
        let mut b = Vec::new();
        b.extend_from_slice(&u32::try_from(size).unwrap().to_be_bytes());
        b.extend_from_slice(b"ftyp");
        b.extend_from_slice(major);
        b.extend_from_slice(&[0, 0, 2, 0]);
        for brand in compatible {
            b.extend_from_slice(*brand);
        }
        b
    }

    #[test]
    fn mp4() {
        assert!(matches_mp4(&ftyp(b"mp42", &[])));
        assert!(matches_mp4(&ftyp(b"isom", &[b"iso2", b"mp41"])));
        assert_eq!(unknown(&ftyp(b"isom", &[b"avc1", b"mp41"])), "video/mp4");
        assert!(!matches_mp4(&ftyp(b"isom", &[b"iso2", b"avc1"])));
        // The version is skipped: "mp4" there is no brand.
        let mut version = ftyp(b"isom", &[b"avc1"]);
        version[12..15].copy_from_slice(b"mp4");
        assert!(!matches_mp4(&version));
        // A box size that is not a multiple of four, or larger than what
        // is there.
        let mut odd = ftyp(b"mp42", &[]);
        odd[3] = 17;
        odd.push(0);
        assert!(!matches_mp4(&odd));
        let mut long = ftyp(b"isom", &[b"mp41"]);
        long[3] = 24;
        assert!(!matches_mp4(&long));
        assert!(!matches_mp4(&ftyp(b"mp42", &[])[..11]));
        let mut not_ftyp = ftyp(b"mp42", &[]);
        not_ftyp[4] = b'F';
        assert!(!matches_mp4(&not_ftyp));
    }

    /// An EBML header holding a `DocType` element whose data is
    /// `doc_type`, its size written as a vint of `vint_length` bytes.
    fn ebml(doc_type: &[u8], vint_length: usize) -> Vec<u8> {
        let mut b = vec![0x1A, 0x45, 0xDF, 0xA3, 0x9F, 0x42, 0x86, 0x81, 0x01];
        b.extend_from_slice(&[0x42, 0x82]);
        let size = doc_type.len() as u64;
        let marker = 1u64 << (7 * vint_length);
        let vint = (marker | size).to_be_bytes();
        b.extend_from_slice(&vint[8 - vint_length..]);
        b.extend_from_slice(doc_type);
        b.extend_from_slice(&[0x42, 0x87, 0x81, 0x04, 0x00, 0x00, 0x00, 0x00]);
        b
    }

    #[test]
    fn webm() {
        assert!(matches_webm(&ebml(b"webm", 1)));
        assert!(matches_webm(&ebml(b"webm", 2)));
        assert!(matches_webm(&ebml(b"\0\0webm", 1)));
        assert_eq!(unknown(&ebml(b"webm", 1)), "video/webm");
        assert!(!matches_webm(&ebml(b"matroska", 1)));
        assert!(!matches_webm(&ebml(b"webmx", 1)));
        assert!(!matches_webm(&ebml(b"x\0webm", 1)));
        // Cut off inside the element.
        let whole = ebml(b"webm", 1);
        assert!(!matches_webm(&whole[..15]));
        // A DocType past the first 38 bytes is not looked for.
        let mut late = vec![0x1A, 0x45, 0xDF, 0xA3];
        late.extend_from_slice(&[0xEC; 36]);
        late.extend_from_slice(&[0x42, 0x82, 0x84]);
        late.extend_from_slice(b"webm\0\0\0\0");
        assert!(!matches_webm(&late));
        late.drain(4..6);
        assert!(!matches_webm(&late), "at 38");
        late.remove(4);
        assert!(matches_webm(&late), "at 37");
    }

    #[test]
    fn vints() {
        assert_eq!(parse_vint(&[0x84], 0), (1, 4));
        assert_eq!(parse_vint(&[0x00, 0x40, 0x04], 1), (2, 4));
        assert_eq!(parse_vint(&[0x01, 0, 0, 0, 0, 0, 0, 7], 0), (8, 7));
        // Cut off: what was read.
        assert_eq!(parse_vint(&[0x00, 0x20, 0x01], 1), (3, 1));
    }

    /// A layer III frame header: version bits, bit-rate index, sample-rate
    /// index, padding.
    fn mp3_header(version: u8, bit_rate: u8, sample_rate: u8, pad: bool) -> [u8; 4] {
        [
            0xFF,
            0xE0 | version << 3 | 0b01 << 1 | 1,
            bit_rate << 4 | sample_rate << 2 | u8::from(pad) << 1,
            0xC4,
        ]
    }

    fn mp3(first: [u8; 4], gap: usize, second: [u8; 4]) -> Vec<u8> {
        let mut b = first.to_vec();
        b.resize(gap, 0x55);
        b.extend_from_slice(&second);
        b
    }

    #[test]
    fn mp3_without_id3() {
        // Version 3 reads the mp2.5 rates: index 9 is 80000, and
        // 80000 * 144 / 44100 is 261.
        let frame = mp3_header(3, 9, 0, false);
        assert_eq!(match_mp3_header(&frame, 0), Some(44_100));
        assert_eq!(mp3_frame_size(&parse_mp3_frame(&frame, 0), 44_100), 261);
        assert!(matches_mp3_without_id3(&mp3(frame, 261, frame)));
        assert_eq!(unknown(&mp3(frame, 261, frame)), "audio/mpeg");
        assert!(!matches_mp3_without_id3(&mp3(frame, 260, frame)));
        assert!(!matches_mp3_without_id3(&mp3(frame, 261, frame)[..264]));
        // Padding adds a byte; version 2 reads the mp3 rates, and version
        // 1 scales by 72.
        let padded = mp3_header(3, 9, 0, true);
        assert!(matches_mp3_without_id3(&mp3(padded, 262, frame)));
        let v2 = mp3_header(2, 1, 1, false);
        assert_eq!(mp3_frame_size(&parse_mp3_frame(&v2, 0), 48_000), 96);
        assert!(matches_mp3_without_id3(&mp3(v2, 96, v2)));
        let v1 = mp3_header(1, 14, 2, false);
        assert_eq!(mp3_frame_size(&parse_mp3_frame(&v1, 0), 32_000), 360);
        // A header the matcher refuses: no sync, layer I, bit rate 15,
        // sample rate 3.
        // A sync is both 0xFF and the next byte's top three bits; the
        // rest of these headers is valid.
        let mut no_sync = frame;
        no_sync[1] &= !0xE0;
        assert_eq!(match_mp3_header(&no_sync, 0), None);
        let mut no_sync = frame;
        no_sync[0] = 0xFE;
        assert_eq!(match_mp3_header(&no_sync, 0), None);
        let mut layer_one = frame;
        layer_one[1] |= 0b11 << 1;
        assert_eq!(match_mp3_header(&layer_one, 0), None);
        assert_eq!(match_mp3_header(&mp3_header(3, 15, 0, false), 0), None);
        assert_eq!(match_mp3_header(&mp3_header(3, 9, 3, false), 0), None);
        // Bit rate 0 is a frame size below 4.
        let free = mp3_header(3, 0, 0, false);
        assert!(!matches_mp3_without_id3(&mp3(free, 4, free)));
    }

    #[test]
    fn archives() {
        let cases: [(&[u8], &str); 3] = [
            (&[0x1F, 0x8B, 0x08, 0x00], "application/x-gzip"),
            (b"PK\x03\x04\x14\x00", "application/zip"),
            (b"Rar!\x1A\x07\x00\xCF", "application/x-rar-compressed"),
        ];
        for (input, expected) in cases {
            assert_eq!(archive_type(input), Some(expected), "{input:?}");
            assert_eq!(unknown(input), expected, "{input:?}");
        }
        assert_eq!(archive_type(b"Rar!\x1A\x07\x01"), None);
    }

    #[test]
    fn binary_data_bytes() {
        let binary: Vec<u8> = (0..=0xFF).filter(|&b| is_binary_data_byte(b)).collect();
        let expected: Vec<u8> = (0x00..=0x08)
            .chain([0x0B])
            .chain(0x0E..=0x1A)
            .chain(0x1C..=0x1F)
            .collect();
        assert_eq!(binary, expected);
        for b in 0..=0xFF_u8 {
            let header = [b'a', b];
            let want = if is_binary_data_byte(b) {
                "application/octet-stream"
            } else {
                "text/plain"
            };
            assert_eq!(unknown(&header), want, "{b:#04x}");
            assert_eq!(text_or_binary(&header).to_string(), want, "{b:#04x}");
        }
    }

    #[test]
    fn only_the_resource_header_is_read() {
        let mut body = vec![b'a'; RESOURCE_HEADER_LENGTH + 1];
        body[RESOURCE_HEADER_LENGTH] = 0x00;
        assert_eq!(computed(None, false, false, &body), "text/plain");
        assert_eq!(identify_unknown(&body, false).to_string(), "text/plain");
        assert_eq!(text_or_binary(&body).to_string(), "text/plain");
        assert_eq!(
            computed(Some("text/plain"), true, false, &body),
            "text/plain"
        );
        body[RESOURCE_HEADER_LENGTH - 1] = 0x00;
        assert_eq!(
            computed(None, false, false, &body),
            "application/octet-stream"
        );
        assert_eq!(
            computed(Some("text/plain"), true, false, &body),
            "application/octet-stream"
        );
        assert_eq!(computed(None, false, false, b""), "text/plain");
    }

    #[test]
    fn leading_whitespace_cannot_run_a_pattern_past_the_end() {
        // Long enough for "<A " before the skip, too short after it.
        assert_eq!(unknown(b"   <A"), "text/plain");
        assert_eq!(unknown(b"          "), "text/plain");
    }
}
