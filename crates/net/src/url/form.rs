//! A form's entries as a request carries them (HTML section 4.10.21.8,
//! URL Standard section 5, M26.9): the `application/x-www-form-urlencoded`
//! serializer, which a GET puts in the action URL's query and a POST in
//! its body; `multipart/form-data`; and `text/plain`. Each encodes the
//! names and values in the form's encoding, a character it cannot
//! represent as a numeric character reference, and makes their line
//! breaks CR LF.

use encoding::Encoding;

use super::percent::{EncodeSet, percent_encode_after_encoding};

/// An entry of a form's entry list: a name, and a string or a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormEntry {
    pub name: String,
    pub value: FormValue,
}

/// An entry's value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormValue {
    Text(String),
    /// A file input's: its name and type, with its contents. One with no
    /// file chosen is the empty file named "" of type
    /// `application/octet-stream`.
    File {
        name: String,
        content_type: String,
        data: Vec<u8>,
    },
}

impl FormEntry {
    /// A string entry.
    pub fn text(name: &str, value: &str) -> FormEntry {
        FormEntry {
            name: name.to_owned(),
            value: FormValue::Text(value.to_owned()),
        }
    }
}

/// `text` with each CR LF, lone CR and lone LF made CR LF (HTML's
/// "normalize line breaks").
fn crlf(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r\n")
}

/// The name and string value of an entry for the serializers that send
/// no files: a file's is its name.
fn pair(entry: &FormEntry) -> (String, String) {
    let value = match &entry.value {
        FormValue::Text(text) => text.as_str(),
        FormValue::File { name, .. } => name.as_str(),
    };
    (crlf(&entry.name), crlf(value))
}

/// The `application/x-www-form-urlencoded` serializer (URL Standard
/// section 5.2): each name and value percent-encoded after encoding with
/// `encoding`'s output encoding, in the form set, a space as `+`, joined
/// by `=` and `&`.
pub fn urlencoded(entries: &[FormEntry], encoding: Encoding) -> String {
    let encoding = encoding.output_encoding();
    let mut out = String::new();
    for (i, entry) in entries.iter().enumerate() {
        let (name, value) = pair(entry);
        if i > 0 {
            out.push('&');
        }
        percent_encode_after_encoding(encoding, &name, EncodeSet::Form, &mut out);
        out.push('=');
        percent_encode_after_encoding(encoding, &value, EncodeSet::Form, &mut out);
    }
    out
}

/// `text` in `encoding`, a character it cannot represent as `&#N;`, as
/// HTML's "encode" for forms has it; UTF-8 for an encoding with no
/// encoder yet.
fn encoded(text: &str, encoding: Encoding) -> Vec<u8> {
    encoding::encode(text, encoding).0
}

/// A name or a file name in a `Content-Disposition` field: line feed,
/// carriage return and the quotation mark escaped as HTML says.
fn disposition(name: &str) -> String {
    name.replace('\n', "%0A")
        .replace('\r', "%0D")
        .replace('"', "%22")
}

/// The `multipart/form-data` encoding algorithm (HTML section
/// 4.10.21.8, RFC 7578) with `boundary`: a part an entry, its name, and a
/// file's name and type, in its `Content-Disposition` and `Content-Type`.
pub fn multipart(entries: &[FormEntry], encoding: Encoding, boundary: &str) -> Vec<u8> {
    let encoding = encoding.output_encoding();
    let mut out = Vec::new();
    for entry in entries {
        out.extend_from_slice(b"--");
        out.extend_from_slice(boundary.as_bytes());
        out.extend_from_slice(b"\r\nContent-Disposition: form-data; name=\"");
        out.extend(encoded(&disposition(&crlf(&entry.name)), encoding));
        out.push(b'"');
        match &entry.value {
            FormValue::Text(text) => {
                out.extend_from_slice(b"\r\n\r\n");
                out.extend(encoded(&crlf(text), encoding));
            }
            FormValue::File {
                name,
                content_type,
                data,
            } => {
                out.extend_from_slice(b"; filename=\"");
                out.extend(encoded(&disposition(name), encoding));
                out.extend_from_slice(b"\"\r\nContent-Type: ");
                out.extend_from_slice(content_type.as_bytes());
                out.extend_from_slice(b"\r\n\r\n");
                out.extend_from_slice(data);
            }
        }
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"--");
    out.extend_from_slice(boundary.as_bytes());
    out.extend_from_slice(b"--\r\n");
    out
}

/// The `text/plain` encoding algorithm (HTML section 4.10.21.8): a line
/// an entry, `name=value`, encoded.
pub fn text_plain(entries: &[FormEntry], encoding: Encoding) -> Vec<u8> {
    let mut text = String::new();
    for entry in entries {
        let (name, value) = pair(entry);
        text.push_str(&name);
        text.push('=');
        text.push_str(&value);
        text.push_str("\r\n");
    }
    encoded(&text, encoding.output_encoding())
}

/// A boundary for a `multipart/form-data` body: `----SilverFormBoundary`
/// and 16 letters and digits from `seed`, the reference browser's form.
pub fn boundary(seed: u64) -> String {
    const ALPHABET: &[u8; 62] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut out = String::from("----SilverFormBoundary");
    for _ in 0..16 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.push(ALPHABET[(state % 62) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use encoding::{UTF_8, WINDOWS_1252};

    fn entries() -> Vec<FormEntry> {
        vec![
            FormEntry::text("q", "a b&c=d"),
            FormEntry::text("line\nbreak", "x\ry\r\nz"),
            FormEntry::text("e", "\u{e9}\u{20ac}\u{65e5}"),
            FormEntry::text("sym", "*-._~!'()+,/:"),
        ]
    }

    #[test]
    fn urlencoded_is_the_url_standards() {
        assert_eq!(
            urlencoded(&entries(), UTF_8),
            "q=a+b%26c%3Dd&line%0D%0Abreak=x%0D%0Ay%0D%0Az\
             &e=%C3%A9%E2%82%AC%E6%97%A5&sym=*-._%7E%21%27%28%29%2B%2C%2F%3A"
        );
        // windows-1252 has the e and the euro, not the ideograph.
        assert_eq!(
            urlencoded(&entries()[2..3], WINDOWS_1252),
            "e=%E9%80%26%2326085%3B"
        );
    }

    #[test]
    fn multipart_parts_a_part_an_entry() {
        let body = multipart(
            &[
                FormEntry::text("na\"me", "one\ntwo"),
                FormEntry {
                    name: "f".to_owned(),
                    value: FormValue::File {
                        name: String::new(),
                        content_type: "application/octet-stream".to_owned(),
                        data: Vec::new(),
                    },
                },
            ],
            UTF_8,
            "B",
        );
        assert_eq!(
            String::from_utf8(body).unwrap(),
            "--B\r\nContent-Disposition: form-data; name=\"na%22me\"\r\n\r\none\r\ntwo\r\n\
             --B\r\nContent-Disposition: form-data; name=\"f\"; filename=\"\"\r\n\
             Content-Type: application/octet-stream\r\n\r\n\r\n--B--\r\n"
        );
    }

    #[test]
    fn text_plain_is_a_line_an_entry() {
        assert_eq!(
            text_plain(&entries()[..2], UTF_8),
            b"q=a b&c=d\r\nline\r\nbreak=x\r\ny\r\nz\r\n"
        );
    }

    #[test]
    fn a_boundary_is_the_prefix_and_sixteen_alphanumerics() {
        let b = boundary(42);
        assert!(b.starts_with("----SilverFormBoundary"));
        let tail = &b["----SilverFormBoundary".len()..];
        assert_eq!(tail.len(), 16);
        assert!(tail.bytes().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(boundary(42), boundary(43));
    }
}
