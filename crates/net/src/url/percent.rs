//! Percent-encoded bytes (URL Standard §1.3): the percent-encode sets,
//! UTF-8 percent-encoding, percent-encoding after encoding with a
//! legacy encoding, and percent-decoding.

use encoding::{Encoding, UTF_8};

/// A percent-encode set: the code points the URL parser writes as
/// percent-encoded bytes in one component. Each set contains the one
/// before it; every set contains the C0 controls and everything past
/// U+007E (~).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EncodeSet {
    /// The C0 control percent-encode set: opaque paths and hosts.
    C0Control,
    /// The fragment percent-encode set.
    Fragment,
    /// The query percent-encode set: a non-special URL's query.
    Query,
    /// The special-query percent-encode set: a special URL's query.
    SpecialQuery,
    /// The path percent-encode set.
    Path,
    /// The userinfo percent-encode set: username and password.
    Userinfo,
    // The component and application/x-www-form-urlencoded sets come
    // with their users, scripts and form submission (M25, M27).
}

impl EncodeSet {
    /// Whether `c` is in the set.
    pub(crate) fn contains(self, c: char) -> bool {
        let c = u32::from(c);
        if !(0x20..=0x7E).contains(&c) {
            return true;
        }
        let c = c as u8;
        match self {
            EncodeSet::C0Control => false,
            EncodeSet::Fragment => matches!(c, b' ' | b'"' | b'<' | b'>' | b'`'),
            EncodeSet::Query => matches!(c, b' ' | b'"' | b'#' | b'<' | b'>'),
            EncodeSet::SpecialQuery => EncodeSet::Query.contains(c as char) || c == b'\'',
            EncodeSet::Path => {
                EncodeSet::Query.contains(c as char)
                    || matches!(c, b'?' | b'^' | b'`' | b'{' | b'}')
            }
            EncodeSet::Userinfo => {
                EncodeSet::Path.contains(c as char)
                    || matches!(
                        c,
                        b'/' | b':' | b';' | b'=' | b'@' | b'[' | b'\\' | b']' | b'|'
                    )
            }
        }
    }
}

/// Appends `%XX`, the byte in upper-case hex (*percent-encode*).
pub(crate) fn percent_encode_byte(byte: u8, out: &mut String) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    out.push('%');
    out.push(HEX[usize::from(byte >> 4)] as char);
    out.push(HEX[usize::from(byte & 0xF)] as char);
}

/// *UTF-8 percent-encode* `c` using `set`, appending to `out`.
pub(crate) fn utf8_percent_encode_char(c: char, set: EncodeSet, out: &mut String) {
    if !set.contains(c) {
        out.push(c);
        return;
    }
    let mut buffer = [0; 4];
    for &byte in c.encode_utf8(&mut buffer).as_bytes() {
        percent_encode_byte(byte, out);
    }
}

/// *UTF-8 percent-encode* the string `input` using `set`, appending to
/// `out`.
pub(crate) fn utf8_percent_encode(input: &str, set: EncodeSet, out: &mut String) {
    for c in input.chars() {
        utf8_percent_encode_char(c, set, out);
    }
}

/// *Percent-encode after encoding*: `input` encoded with `encoding`,
/// each byte whose code point is in `set` percent-encoded, and a
/// character the encoding cannot represent written as `%26%23N%3B`
/// (the percent-encoding of `&#N;`), appending to `out`.
///
/// An encoding the `encoding` crate cannot encode yet (a legacy CJK
/// one) encodes as UTF-8. The standard's `spaceAsPlus` belongs to
/// `application/x-www-form-urlencoded`, which comes with its users.
pub(crate) fn percent_encode_after_encoding(
    encoding: Encoding,
    input: &str,
    set: EncodeSet,
    out: &mut String,
) {
    let mut encoder = encoding
        .new_encoder()
        .unwrap_or_else(|| UTF_8.new_encoder().expect("UTF-8 encodes"));
    let mut bytes = Vec::with_capacity(4);
    for c in input.chars() {
        bytes.clear();
        if encoder.encode_char(c, &mut bytes).is_err() {
            out.push_str("%26%23");
            out.push_str(&u32::from(c).to_string());
            out.push_str("%3B");
            continue;
        }
        for &byte in &bytes {
            // Every set contains every non-ASCII code point, so only an
            // ASCII byte can stand for itself.
            if byte < 0x80 && !set.contains(byte as char) {
                out.push(byte as char);
            } else {
                percent_encode_byte(byte, out);
            }
        }
    }
}

/// *Percent-decode* `input`: each `%` followed by two hex digits becomes
/// the byte they spell, and every other byte stays as it is.
pub fn percent_decode(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        let byte = input[i];
        if byte == b'%'
            && let (Some(high), Some(low)) = (
                input.get(i + 1).and_then(|&b| hex_value(b)),
                input.get(i + 2).and_then(|&b| hex_value(b)),
            )
        {
            out.push(high << 4 | low);
            i += 3;
            continue;
        }
        out.push(byte);
        i += 1;
    }
    out
}

fn hex_value(byte: u8) -> Option<u8> {
    (byte as char).to_digit(16).map(|d| d as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_standards_examples() {
        // URL Standard §1.3's table.
        let mut out = String::new();
        percent_encode_byte(0x23, &mut out);
        percent_encode_byte(0x7F, &mut out);
        assert_eq!(out, "%23%7F");
        assert_eq!(percent_decode(b"%25%s%1G"), b"%%s%1G");
        assert_eq!(
            percent_decode("\u{203D}%25%2E".as_bytes()),
            b"\xE2\x80\xBD\x25\x2E"
        );
        let mut out = String::new();
        utf8_percent_encode("Say what\u{203D}", EncodeSet::Userinfo, &mut out);
        assert_eq!(out, "Say%20what%E2%80%BD");
    }

    #[test]
    fn after_encoding_with_a_legacy_encoding() {
        let mut out = String::new();
        percent_encode_after_encoding(
            encoding::WINDOWS_1252,
            "a \u{E9}\u{2603}'",
            EncodeSet::SpecialQuery,
            &mut out,
        );
        assert_eq!(out, "a%20%E9%26%239731%3B%27");
        let mut out = String::new();
        percent_encode_after_encoding(UTF_8, "\u{E9}'", EncodeSet::Query, &mut out);
        assert_eq!(out, "%C3%A9'");
    }

    #[test]
    fn the_sets_nest_as_the_standard_defines_them() {
        // The chain C0 control, query, path, userinfo; special-query is
        // a branch off query (it has ' and the path set does not).
        let pairs = [
            (EncodeSet::C0Control, EncodeSet::Query),
            (EncodeSet::Query, EncodeSet::Path),
            (EncodeSet::Path, EncodeSet::Userinfo),
            (EncodeSet::Query, EncodeSet::SpecialQuery),
            (EncodeSet::C0Control, EncodeSet::Fragment),
        ];
        for (smaller, larger) in pairs {
            for c in (0..=0x7F).filter_map(char::from_u32) {
                assert!(
                    !smaller.contains(c) || larger.contains(c),
                    "{larger:?} {c:?}"
                );
            }
        }
        assert!(EncodeSet::SpecialQuery.contains('\'') && !EncodeSet::Path.contains('\''));
        // The fragment set is not in that chain: it has ` but not #.
        assert!(EncodeSet::Fragment.contains('`') && !EncodeSet::Fragment.contains('#'));
        // What the userinfo set leaves alone, besides alphanumerics.
        let kept: String = (0x20..0x7F)
            .filter_map(char::from_u32)
            .filter(|&c| !EncodeSet::Userinfo.contains(c))
            .filter(|c| !c.is_ascii_alphanumeric())
            .collect();
        assert_eq!(kept, "!$%&'()*+,-._~");
    }
}
