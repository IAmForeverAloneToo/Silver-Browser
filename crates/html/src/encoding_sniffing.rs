//! A document's character encoding from its bytes (HTML §13.2.3): the
//! encoding sniffing algorithm with its prescan for `<meta>` and its
//! XML declaration fallback, what a `<meta>` found while parsing does to
//! a tentative encoding, and extracting a character encoding from a
//! `<meta>`'s `content` (HTML §2.5.3).

use encoding::{Encoding, UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1252, X_USER_DEFINED};

/// How many bytes the prescan reads: the 1024 HTML encourages, the same
/// bound authors must keep their declaration within.
pub const PRESCAN_LENGTH: usize = 1024;

/// How sure the parser is of the encoding it decodes with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// A guess: a `<meta>` found while parsing may change it.
    Tentative,
    /// Settled: a byte order mark, the transport layer, or a `<meta>`
    /// already acted on.
    Certain,
}

/// *The encoding sniffing algorithm*, for bytes that are all in hand:
///
/// 1. a byte order mark, certain;
/// 2. the encoding the transport layer gives (the `Content-Type`
///    charset), certain;
/// 3. what the prescan of the first 1024 bytes finds, tentative;
/// 4. UTF-8, tentative.
///
/// UTF-8 is the default HTML suggests where encodings can be
/// prescribed; the locale defaults it lists for legacy content are not
/// used. The steps for a nested document's parent and for remembered
/// or detected encodings do not apply.
pub fn sniff(bytes: &[u8], transport: Option<Encoding>) -> (Encoding, Confidence) {
    if let Some((encoding, _)) = encoding::bom_sniff(bytes) {
        return (encoding, Confidence::Certain);
    }
    if let Some(encoding) = transport {
        return (encoding, Confidence::Certain);
    }
    match prescan(bytes) {
        Some(encoding) => (encoding, Confidence::Tentative),
        None => (UTF_8, Confidence::Tentative),
    }
}

/// *Changing the encoding while parsing* (§13.2.3.4), for a `<meta>`
/// naming `new` while the parser decodes with `current` and its
/// confidence is tentative. `None` means the confidence becomes certain
/// and nothing changes; `Some` gives the encoding to decode the bytes
/// with again, certain, and parse from the start.
///
/// The standard lets a parser switch decoders on the fly when what was
/// decoded so far reads the same in both encodings. This parser has the
/// whole body and always starts over, which the standard also allows.
pub fn change_the_encoding(current: Encoding, new: Encoding) -> Option<Encoding> {
    if is_utf16(current) {
        return None;
    }
    let new = if is_utf16(new) {
        UTF_8
    } else if new == X_USER_DEFINED {
        WINDOWS_1252
    } else {
        new
    };
    (new != current).then_some(new)
}

/// UTF-16BE/LE: either UTF-16 encoding.
fn is_utf16(encoding: Encoding) -> bool {
    encoding == UTF_16BE || encoding == UTF_16LE
}

/// *Prescan a byte stream to determine its encoding*, over the first
/// [`PRESCAN_LENGTH`] bytes: the encoding a `<meta charset>` or a
/// `<meta http-equiv=content-type content=…>` declares, skipping
/// comments and other tags; or, when the bytes run out first, what an
/// XML declaration says.
pub fn prescan(bytes: &[u8]) -> Option<Encoding> {
    let bytes = &bytes[..bytes.len().min(PRESCAN_LENGTH)];
    // The scan only ends by finding an encoding or by running out of
    // bytes, which is where the XML declaration is looked at.
    scan_for_meta(bytes).or_else(|| get_xml_encoding(bytes))
}

/// HTML's whitespace bytes, as the prescan names them: TAB, LF, FF, CR
/// and SP.
fn is_space(b: u8) -> bool {
    matches!(b, 0x09 | 0x0A | 0x0C | 0x0D | 0x20)
}

/// The byte, lower-cased if it is an ASCII capital, as the code point of
/// the same value.
fn lowered(b: u8) -> char {
    char::from(b.to_ascii_lowercase())
}

/// What the prescan has made of a `<meta>`'s `charset` so far.
#[derive(Clone, Copy, PartialEq)]
enum Charset {
    /// Nothing named one: distinct from a label that names nothing.
    Null,
    /// A label that names no encoding.
    Failure,
    Found(Encoding),
}

/// The prescan's main loop. `None` when the bytes run out, at whatever
/// step that happens.
fn scan_for_meta(bytes: &[u8]) -> Option<Encoding> {
    // UTF-16 XML declarations: '<?x' in either byte order.
    if bytes.starts_with(&[0x3C, 0x00, 0x3F, 0x00, 0x78, 0x00]) {
        return Some(UTF_16LE);
    }
    if bytes.starts_with(&[0x00, 0x3C, 0x00, 0x3F, 0x00, 0x78]) {
        return Some(UTF_16BE);
    }
    let mut position = 0;
    loop {
        let rest = bytes.get(position..).filter(|rest| !rest.is_empty())?;
        if rest.starts_with(b"<!--") {
            // To the first '>' with two '-' before it; those may be the
            // comment opener's own.
            position += 4;
            while !(bytes.get(position)? == &b'>' && &bytes[position - 2..position] == b"--") {
                position += 1;
            }
        } else if rest.len() >= 6
            && rest[..5].eq_ignore_ascii_case(b"<meta")
            && (is_space(rest[5]) || rest[5] == b'/')
        {
            position += 5;
            if let Some(encoding) = meta_declaration(bytes, &mut position)? {
                return Some(encoding);
            }
        } else if rest[0] == b'<' && starts_a_tag_name(&rest[1..]) {
            while !(is_space(*bytes.get(position)?) || bytes[position] == b'>') {
                position += 1;
            }
            while get_an_attribute(bytes, &mut position)?.is_some() {}
        } else if rest.starts_with(b"<!") || rest.starts_with(b"</") || rest.starts_with(b"<?") {
            position += 1;
            while *bytes.get(position)? != b'>' {
                position += 1;
            }
        }
        // Next byte.
        position += 1;
    }
}

/// Whether the bytes after a `<` begin a start or end tag's name: an
/// optional `/` and an ASCII letter.
fn starts_a_tag_name(after: &[u8]) -> bool {
    let after = after.strip_prefix(b"/").unwrap_or(after);
    after.first().is_some_and(u8::is_ascii_alphabetic)
}

/// The attributes of a `<meta>` from `position` on, and the encoding
/// they declare, if any (the prescan's steps for `<meta`). The outer
/// `None` is running out of bytes.
fn meta_declaration(bytes: &[u8], position: &mut usize) -> Option<Option<Encoding>> {
    let mut attribute_list: Vec<String> = Vec::new();
    let mut got_pragma = false;
    let mut need_pragma: Option<bool> = None;
    let mut charset = Charset::Null;
    while let Some((name, value)) = get_an_attribute(bytes, position)? {
        if attribute_list.contains(&name) {
            continue;
        }
        match name.as_str() {
            "http-equiv" => got_pragma |= value == "content-type",
            "content" => {
                if charset == Charset::Null
                    && let Some(encoding) = extract_from_meta_content(&value)
                {
                    charset = Charset::Found(encoding);
                    need_pragma = Some(true);
                }
            }
            "charset" => {
                charset = Encoding::for_label(&value).map_or(Charset::Failure, Charset::Found);
                need_pragma = Some(false);
            }
            _ => {}
        }
        attribute_list.push(name);
    }
    // Processing.
    let Some(need_pragma) = need_pragma else {
        return Some(None);
    };
    if need_pragma && !got_pragma {
        return Some(None);
    }
    let Charset::Found(charset) = charset else {
        return Some(None);
    };
    Some(Some(if is_utf16(charset) {
        UTF_8
    } else if charset == X_USER_DEFINED {
        WINDOWS_1252
    } else {
        charset
    }))
}

/// *Get an attribute* at `position`: its name and value, lower-cased,
/// or `Some(None)` at a `>` where there is none. `None` is running out
/// of bytes.
fn get_an_attribute(bytes: &[u8], position: &mut usize) -> Option<Option<(String, String)>> {
    let byte = |position: &usize| bytes.get(*position).copied();
    while byte(position).is_some_and(|b| is_space(b) || b == b'/') {
        *position += 1;
    }
    if byte(position)? == b'>' {
        return Some(None);
    }
    let mut name = String::new();
    let mut value = String::new();
    // The name, up to '=', a space, '/' or '>'. An '=' first is part of
    // the name.
    loop {
        let b = byte(position)?;
        if b == b'=' && !name.is_empty() {
            *position += 1;
            break;
        }
        if is_space(b) {
            // Spaces, and then '=' or the end of the attribute.
            while is_space(byte(position)?) {
                *position += 1;
            }
            if byte(position)? != b'=' {
                return Some(Some((name, value)));
            }
            *position += 1;
            break;
        }
        if b == b'/' || b == b'>' {
            return Some(Some((name, value)));
        }
        name.push(lowered(b));
        *position += 1;
    }
    // The value.
    while is_space(byte(position)?) {
        *position += 1;
    }
    let first = byte(position)?;
    match first {
        b'"' | b'\'' => loop {
            *position += 1;
            let b = byte(position)?;
            if b == first {
                *position += 1;
                return Some(Some((name, value)));
            }
            value.push(lowered(b));
        },
        b'>' => return Some(Some((name, value))),
        _ => {
            value.push(lowered(first));
            *position += 1;
        }
    }
    loop {
        let b = byte(position)?;
        if is_space(b) || b == b'>' {
            return Some(Some((name, value)));
        }
        value.push(lowered(b));
        *position += 1;
    }
}

/// *Get an XML encoding*: the encoding an `<?xml … encoding="…"?>`
/// declaration at the very start names, UTF-16 read as UTF-8.
fn get_xml_encoding(bytes: &[u8]) -> Option<Encoding> {
    if !bytes.starts_with(b"<?xml") {
        return None;
    }
    let declaration_end = bytes.iter().position(|&b| b == b'>')?;
    let mut position = bytes[..declaration_end]
        .windows(8)
        .position(|window| window == b"encoding")?
        + 8;
    while *bytes.get(position)? <= 0x20 {
        position += 1;
    }
    if bytes[position] != b'=' {
        return None;
    }
    position += 1;
    while *bytes.get(position)? <= 0x20 {
        position += 1;
    }
    let quote = bytes[position];
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    position += 1;
    let length = bytes[position..].iter().position(|&b| b == quote)?;
    let label = &bytes[position..position + length];
    if label.iter().any(|&b| b <= 0x20) {
        return None;
    }
    let encoding = Encoding::for_label_bytes(label)?;
    Some(if is_utf16(encoding) { UTF_8 } else { encoding })
}

/// *The algorithm for extracting a character encoding from a meta
/// element* (§2.5.3), given its `content`: the encoding the first
/// `charset=` names, its value quoted or up to whitespace or `;`.
pub fn extract_from_meta_content(content: &str) -> Option<Encoding> {
    // Every delimiter is ASCII, so byte offsets fall on character
    // boundaries.
    let s = content.as_bytes();
    let mut position = 0;
    loop {
        let found = s
            .get(position..)?
            .windows(7)
            .position(|window| window.eq_ignore_ascii_case(b"charset"))?;
        let mut next = position + found + 7;
        while s.get(next).is_some_and(u8::is_ascii_whitespace) {
            next += 1;
        }
        if s.get(next) != Some(&b'=') {
            position = next;
            continue;
        }
        next += 1;
        while s.get(next).is_some_and(u8::is_ascii_whitespace) {
            next += 1;
        }
        return match *s.get(next)? {
            quote @ (b'"' | b'\'') => {
                let length = s[next + 1..].iter().position(|&b| b == quote)?;
                Encoding::for_label(&content[next + 1..next + 1 + length])
            }
            _ => {
                let length = s[next..]
                    .iter()
                    .position(|&b| b.is_ascii_whitespace() || b == b';')
                    .unwrap_or(s.len() - next);
                Encoding::for_label(&content[next..next + length])
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use encoding::{GBK, KOI8_R, REPLACEMENT};

    #[test]
    fn the_sniffing_order() {
        let meta = b"<meta charset=koi8-r>";
        assert_eq!(sniff(meta, None), (KOI8_R, Confidence::Tentative));
        assert_eq!(sniff(meta, Some(GBK)), (GBK, Confidence::Certain));
        let bom = b"\xEF\xBB\xBF<meta charset=koi8-r>";
        assert_eq!(sniff(bom, Some(GBK)), (UTF_8, Confidence::Certain));
        assert_eq!(sniff(b"\xFF\xFEx", None), (UTF_16LE, Confidence::Certain));
        assert_eq!(sniff(b"<p>caf\xE9", None), (UTF_8, Confidence::Tentative));
    }

    #[test]
    fn changing_the_encoding() {
        assert_eq!(change_the_encoding(UTF_8, KOI8_R), Some(KOI8_R));
        assert_eq!(change_the_encoding(UTF_8, UTF_8), None);
        // UTF-16 declared in an ASCII-compatible document is UTF-8.
        assert_eq!(change_the_encoding(UTF_8, UTF_16LE), None);
        assert_eq!(change_the_encoding(KOI8_R, UTF_16BE), Some(UTF_8));
        assert_eq!(
            change_the_encoding(UTF_8, X_USER_DEFINED),
            Some(WINDOWS_1252)
        );
        // Once in UTF-16, nothing a <meta> says counts.
        assert_eq!(change_the_encoding(UTF_16BE, KOI8_R), None);
        assert_eq!(change_the_encoding(UTF_16LE, UTF_8), None);
    }

    fn scan(bytes: &[u8]) -> Option<&'static str> {
        prescan(bytes).map(Encoding::name)
    }

    #[test]
    fn meta_charset() {
        assert_eq!(scan(b"<meta charset=koi8-r>"), Some("KOI8-R"));
        assert_eq!(scan(b"<META CHARSET=\"KOI8-R\">"), Some("KOI8-R"));
        assert_eq!(scan(b"<meta charset='  latin1 '>"), Some("windows-1252"));
        assert_eq!(scan(b"<meta/charset=latin2>"), Some("ISO-8859-2"));
        assert_eq!(
            scan(b"<meta  name=x charset = cp1251 >"),
            Some("windows-1251")
        );
        assert_eq!(
            scan(b"<!doctype html><html><head><meta charset=utf-8>"),
            Some("UTF-8")
        );
        // UTF-16 and x-user-defined are not what an ASCII <meta> means.
        assert_eq!(scan(b"<meta charset=utf-16be>"), Some("UTF-8"));
        assert_eq!(scan(b"<meta charset=x-user-defined>"), Some("windows-1252"));
        // The replacement encoding is an encoding like any other.
        assert_eq!(
            scan(b"<meta charset=iso-2022-kr>"),
            Some(REPLACEMENT.name())
        );
        // `<meta` wants a space or `/` after it.
        assert_eq!(scan(b"<metacharset=koi8-r>"), None);
        assert_eq!(scan(b"<meta>"), None);
    }

    #[test]
    fn a_label_that_names_nothing_lets_the_scan_go_on() {
        assert_eq!(
            scan(b"<meta charset=bogus><meta charset=koi8-r>"),
            Some("KOI8-R")
        );
        // ...but within one <meta>, a failed charset is not replaced by
        // a later content.
        assert_eq!(
            scan(b"<meta charset=bogus http-equiv=content-type content='charset=koi8-r'>"),
            None
        );
    }

    #[test]
    fn http_equiv_and_content() {
        let both =
            b"<meta http-equiv=\"Content-Type\" content=\"text/html; charset=windows-1251\">";
        assert_eq!(scan(both), Some("windows-1251"));
        let reversed = b"<meta content='text/html;charset=koi8-r' http-equiv=content-type>";
        assert_eq!(scan(reversed), Some("KOI8-R"));
        // Without the pragma, content declares nothing.
        assert_eq!(scan(b"<meta content='text/html; charset=koi8-r'>"), None);
        assert_eq!(
            scan(b"<meta http-equiv=refresh content='charset=koi8-r'>"),
            None
        );
        // A charset attribute wins over content, and needs no pragma.
        assert_eq!(
            scan(b"<meta content='charset=koi8-r' charset=latin2>"),
            Some("ISO-8859-2")
        );
        assert_eq!(
            scan(b"<meta charset=latin2 content='charset=koi8-r' http-equiv=content-type>"),
            Some("ISO-8859-2")
        );
    }

    #[test]
    fn only_the_first_of_a_repeated_attribute_counts() {
        assert_eq!(
            scan(b"<meta charset=koi8-r charset=latin2>"),
            Some("KOI8-R")
        );
        assert_eq!(
            scan(b"<meta http-equiv=content-type http-equiv=x content='charset=koi8-r'>"),
            Some("KOI8-R")
        );
    }

    #[test]
    fn what_the_scan_skips() {
        assert_eq!(
            scan(b"<!-- <meta charset=koi8-r> --><meta charset=latin2>"),
            Some("ISO-8859-2")
        );
        // A comment ends at "-->", not at the first '>'.
        assert_eq!(
            scan(b"<!-- a > b <meta charset=koi8-r> --><meta charset=latin2>"),
            Some("ISO-8859-2")
        );
        assert_eq!(scan(b"<!-- <meta charset=koi8-r> -- >"), None);
        // The comment's own dashes can close it.
        assert_eq!(scan(b"<!--><meta charset=koi8-r>"), Some("KOI8-R"));
        assert_eq!(scan(b"<!---><meta charset=koi8-r>"), Some("KOI8-R"));
        // Inside another tag's attribute, a <meta> is a value.
        assert_eq!(
            scan(b"<a title='<meta charset=koi8-r>'><meta charset=latin2>"),
            Some("ISO-8859-2")
        );
        assert_eq!(scan(b"</a title='<meta charset=koi8-r>'>"), None);
        assert_eq!(
            scan(b"<? <meta charset=koi8-r> ><meta charset=latin2>"),
            Some("ISO-8859-2")
        );
        assert_eq!(scan(b"<!x <meta charset=koi8-r>"), None);
        // Neither a tag nor markup: `< meta` is text.
        assert_eq!(scan(b"< <meta charset=koi8-r>"), Some("KOI8-R"));
    }

    #[test]
    fn only_the_first_1024_bytes_are_read() {
        let late = |at: usize| {
            let mut bytes = vec![b' '; at];
            bytes.extend_from_slice(b"<meta charset=koi8-r>");
            bytes
        };
        let fits = PRESCAN_LENGTH - b"<meta charset=koi8-r>".len();
        assert_eq!(scan(&late(fits)), Some("KOI8-R"));
        // Cut off before its '>', the value is never finished.
        assert_eq!(scan(&late(fits + 1)), None);
        assert_eq!(scan(&late(PRESCAN_LENGTH)), None);
        // A quoted value cut off ends the scan too.
        assert_eq!(scan(b"<meta charset='koi8-r"), None);
    }

    #[test]
    fn xml_declarations() {
        assert_eq!(
            scan(b"<?xml version='1.0' encoding='koi8-r'?>"),
            Some("KOI8-R")
        );
        assert_eq!(scan(b"<?xml encoding = \"utf-16\"?>"), Some("UTF-8"));
        assert_eq!(scan(b"<?xml encoding=\"koi8-r \"?>"), None);
        assert_eq!(scan(b"<?xml encoding=koi8-r?>"), None);
        assert_eq!(scan(b" <?xml encoding='koi8-r'?>"), None);
        assert_eq!(scan(b"<?xml?> encoding='koi8-r'"), None);
        // The declaration is only read when the scan finds no <meta>.
        assert_eq!(
            scan(b"<?xml encoding='koi8-r'?><meta charset=latin2>"),
            Some("ISO-8859-2")
        );
        assert_eq!(scan(b"<\x00?\x00x\x00"), Some("UTF-16LE"));
        assert_eq!(scan(b"\x00<\x00?\x00x"), Some("UTF-16BE"));
    }

    #[test]
    fn content_attribute_values() {
        let extract = |s: &str| extract_from_meta_content(s).map(Encoding::name);
        assert_eq!(
            extract("text/html; charset=windows-1251"),
            Some("windows-1251")
        );
        assert_eq!(extract("text/html;CHARSET = 'koi8-r' ;x"), Some("KOI8-R"));
        assert_eq!(extract("charset=\"latin2\""), Some("ISO-8859-2"));
        assert_eq!(extract("charset=koi8-r;latin2"), Some("KOI8-R"));
        assert_eq!(extract("charset=koi8-r latin2"), Some("KOI8-R"));
        // A charset without '=' is passed over for a later one.
        assert_eq!(extract("charsets; charset=latin2"), Some("ISO-8859-2"));
        assert_eq!(extract("charsetcharset=latin2"), Some("ISO-8859-2"));
        assert_eq!(extract("charset=\"koi8-r"), None);
        assert_eq!(extract("charset="), None);
        assert_eq!(extract("charset= "), None);
        assert_eq!(extract("charset=bogus"), None);
        assert_eq!(extract("text/html"), None);
        assert_eq!(extract("chars"), None);
        assert_eq!(extract("é charset=koi8-r"), Some("KOI8-R"));
    }
}
