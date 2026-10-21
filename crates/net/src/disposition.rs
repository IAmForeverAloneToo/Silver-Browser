//! `Content-Disposition` in HTTP (RFC 6266, M37): whether a response is
//! to be saved rather than shown, and the file name it suggests, from
//! `filename*` (RFC 8187: UTF-8 or ISO-8859-1, percent-encoded) over
//! `filename` (a token or a quoted string, UTF-8 where it decodes as
//! such, else ISO-8859-1). Read leniently, as the reference browser
//! reads what servers send: an unclosed quote ends with the value, a
//! parameter without a value is skipped, and an unknown type is an
//! attachment (RFC 6266 section 4.2). The name is the server's word: the
//! caller makes it safe before it names a file.

/// What a `Content-Disposition` says.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Disposition {
    /// Whether the response is to be saved: `attachment`, or a type
    /// this does not know.
    pub attachment: bool,
    /// The file name it suggests, if one.
    pub filename: Option<String>,
}

/// The longest value read, in bytes: past it, the rest is left out.
pub const MAX_DISPOSITION: usize = 8 << 10;

/// Reads a `Content-Disposition` field's `value`.
pub fn content_disposition(value: &[u8]) -> Disposition {
    let value = &value[..value.len().min(MAX_DISPOSITION)];
    let mut parts = Splitter { rest: value };
    let kind = parts.next_part().unwrap_or_default();
    let kind = String::from_utf8_lossy(trim(&kind)).to_ascii_lowercase();
    let attachment = kind != "inline" && !kind.is_empty();
    let (mut plain, mut extended) = (None, None);
    while let Some(part) = parts.next_part() {
        let part = trim(&part);
        let Some(at) = part.iter().position(|&b| b == b'=') else {
            continue;
        };
        let name = String::from_utf8_lossy(trim(&part[..at])).to_ascii_lowercase();
        let raw = trim(&part[at + 1..]);
        match name.as_str() {
            "filename*" if extended.is_none() => extended = ext_value(&unquote(raw)),
            "filename" if plain.is_none() => plain = Some(text(&unquote(raw))),
            _ => {}
        }
    }
    let filename = extended
        .or(plain)
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty());
    Disposition {
        attachment,
        filename,
    }
}

/// The parts of a value between `;`s, a quoted string's `;` and escaped
/// characters kept within it.
struct Splitter<'a> {
    rest: &'a [u8],
}

impl Splitter<'_> {
    fn next_part(&mut self) -> Option<Vec<u8>> {
        if self.rest.is_empty() {
            return None;
        }
        let (mut quoted, mut escaped) = (false, false);
        for (i, &b) in self.rest.iter().enumerate() {
            match b {
                _ if escaped => escaped = false,
                b'\\' if quoted => escaped = true,
                b'"' => quoted = !quoted,
                b';' if !quoted => {
                    let part = self.rest[..i].to_vec();
                    self.rest = &self.rest[i + 1..];
                    return Some(part);
                }
                _ => {}
            }
        }
        let part = self.rest.to_vec();
        self.rest = &[];
        Some(part)
    }
}

fn trim(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(start, |i| i + 1);
    &bytes[start..end.max(start)]
}

/// A token, or a quoted string's contents with its escapes taken away;
/// an unclosed one ends with the value.
fn unquote(raw: &[u8]) -> Vec<u8> {
    let Some(inner) = raw.strip_prefix(b"\"") else {
        return raw.to_vec();
    };
    let mut out = Vec::with_capacity(inner.len());
    let mut escaped = false;
    for &b in inner {
        match b {
            _ if escaped => {
                out.push(b);
                escaped = false;
            }
            b'\\' => escaped = true,
            b'"' => break,
            _ => out.push(b),
        }
    }
    out
}

/// `bytes` as text: UTF-8 where they decode as such, else ISO-8859-1.
fn text(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes.iter().map(|&b| char::from(b)).collect(),
    }
}

/// An RFC 8187 `ext-value`, `charset'language'value-chars`, decoded; none
/// for another charset or a malformed one.
fn ext_value(raw: &[u8]) -> Option<String> {
    let mut fields = raw.splitn(3, |&b| b == b'\'');
    let charset = String::from_utf8_lossy(fields.next()?).to_ascii_lowercase();
    let _language = fields.next()?;
    let encoded = fields.next()?;
    let mut bytes = Vec::with_capacity(encoded.len());
    let mut i = 0;
    while i < encoded.len() {
        if encoded[i] == b'%' {
            let hex = encoded.get(i + 1..i + 3)?;
            let digit = |b: u8| (b as char).to_digit(16);
            let (hi, lo) = (digit(hex[0])?, digit(hex[1])?);
            bytes.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            bytes.push(encoded[i]);
            i += 1;
        }
    }
    match charset.as_str() {
        "utf-8" => String::from_utf8(bytes).ok(),
        "iso-8859-1" => Some(bytes.iter().map(|&b| char::from(b)).collect()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(value: &str) -> Disposition {
        content_disposition(value.as_bytes())
    }

    fn name(value: &str) -> Option<String> {
        read(value).filename
    }

    #[test]
    fn its_type_says_whether_to_save() {
        assert!(read("attachment").attachment);
        assert!(read("ATTACHMENT; filename=a").attachment);
        assert!(!read("inline").attachment);
        assert!(!read("Inline; filename=a.pdf").attachment);
        // An unknown type is an attachment (RFC 6266 section 4.2); none is
        // no disposition at all.
        assert!(read("x-made-up").attachment);
        assert!(!read("").attachment);
    }

    #[test]
    fn a_name_as_a_token_or_a_quoted_string() {
        assert_eq!(
            name("attachment; filename=report.pdf").as_deref(),
            Some("report.pdf")
        );
        assert_eq!(
            name("attachment; filename=\"a; b \\\"c\\\".txt\"").as_deref(),
            Some("a; b \"c\".txt")
        );
        assert_eq!(
            name("attachment;filename = \"spaced.txt\" ; size=3").as_deref(),
            Some("spaced.txt")
        );
        // An unclosed quote ends with the value.
        assert_eq!(
            name("attachment; filename=\"open.txt").as_deref(),
            Some("open.txt")
        );
        assert_eq!(name("attachment; filename=").as_deref(), None);
        assert_eq!(name("attachment; filename").as_deref(), None);
        assert_eq!(name("attachment").as_deref(), None);
    }

    #[test]
    fn filename_star_comes_first_in_utf_8_or_latin_1() {
        assert_eq!(
            name("attachment; filename=plain.txt; filename*=UTF-8''%E2%82%AC%20rates.txt")
                .as_deref(),
            Some("\u{20ac} rates.txt")
        );
        assert_eq!(
            name("attachment; filename*=iso-8859-1'en'%A3%20rates.txt").as_deref(),
            Some("\u{a3} rates.txt")
        );
        // Another charset, or broken escapes, fall back to filename.
        assert_eq!(
            name("attachment; filename*=koi8-r''%C1; filename=fallback.txt").as_deref(),
            Some("fallback.txt")
        );
        assert_eq!(
            name("attachment; filename*=UTF-8''%E2%8; filename=fallback.txt").as_deref(),
            Some("fallback.txt")
        );
        assert_eq!(
            name("attachment; filename*=UTF-8''%FF; filename=fallback.txt").as_deref(),
            Some("fallback.txt")
        );
    }

    #[test]
    fn a_plain_name_of_raw_bytes_is_utf_8_or_latin_1() {
        let utf8 = content_disposition(b"attachment; filename=\"\xe2\x82\xac.txt\"");
        assert_eq!(utf8.filename.as_deref(), Some("\u{20ac}.txt"));
        let latin1 = content_disposition(b"attachment; filename=\"\xa3.txt\"");
        assert_eq!(latin1.filename.as_deref(), Some("\u{a3}.txt"));
    }

    #[test]
    fn a_long_value_is_read_to_its_bound() {
        let long = format!(
            "attachment; filename=\"{}\"",
            "a".repeat(MAX_DISPOSITION * 2)
        );
        let read = read(&long);
        assert!(read.attachment);
        assert!(
            read.filename
                .is_some_and(|name| name.len() < MAX_DISPOSITION)
        );
    }
}
