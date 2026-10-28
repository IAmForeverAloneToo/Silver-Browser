//! Header lists (Fetch §2.2.2) and the pieces of Fetch §2.2 that read
//! them and MIME types: HTTP whitespace, token code points, collecting an
//! HTTP quoted string, and getting, decoding and splitting a value.
//!
//! A header list holds what arrived. Whether a name is a token and a
//! value free of NUL and stray CR is the HTTP reader's to check (step 5
//! of `docs/design/net.md`'s order of work), before a field gets here.

use std::collections::HashMap;
use std::fmt;

use crate::infra::{isomorphic_decode, isomorphic_encode};

/// A header list: header fields in the order they came, each name and
/// value as bytes. Names match ASCII case-insensitively, and a name may
/// occur more than once.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct HeaderList {
    fields: Vec<(Vec<u8>, Vec<u8>)>,
    /// Each name in the list, lower case, with the casing it first came
    /// with. Appending finds it here rather than by a search through
    /// every field, which would make a response of many fields cost the
    /// square of their number.
    casings: HashMap<Vec<u8>, Vec<u8>>,
}

impl HeaderList {
    /// An empty list.
    pub fn new() -> HeaderList {
        HeaderList::default()
    }

    /// The fields, in order, as `(name, value)`.
    pub fn iter(&self) -> impl Iterator<Item = (&[u8], &[u8])> {
        self.fields
            .iter()
            .map(|(name, value)| (name.as_slice(), value.as_slice()))
    }

    /// How many fields the list holds.
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    /// Whether the list holds no field.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Whether a field named `name` is in the list (*contains*).
    pub fn contains(&self, name: &[u8]) -> bool {
        self.fields
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case(name))
    }

    /// *Get*: the values of every field named `name`, in order, joined by
    /// `, `; or `None` when there is none.
    pub fn get(&self, name: &[u8]) -> Option<Vec<u8>> {
        let mut values = self
            .fields
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_slice());
        let mut joined = values.next()?.to_vec();
        for value in values {
            joined.extend_from_slice(b", ");
            joined.extend_from_slice(value);
        }
        Some(joined)
    }

    /// *Get, decode, and split*: the combined value of the fields named
    /// `name`, isomorphic-decoded and split at each comma outside a
    /// quoted string; or `None` when there is none.
    pub fn get_decode_split(&self, name: &[u8]) -> Option<Vec<String>> {
        self.get(name).map(|value| get_decode_split_value(&value))
    }

    /// The value of the last field named `name`, as it came.
    pub fn last(&self, name: &[u8]) -> Option<&[u8]> {
        self.fields
            .iter()
            .rev()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_slice())
    }

    /// *Append* a field. A name already in the list keeps the casing it
    /// first came with.
    pub fn append(&mut self, name: &[u8], value: &[u8]) {
        let casing = self
            .casings
            .entry(name.to_ascii_lowercase())
            .or_insert_with(|| name.to_vec());
        self.fields.push((casing.clone(), value.to_vec()));
    }

    /// *Delete* every field named `name`.
    pub fn delete(&mut self, name: &[u8]) {
        self.fields.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        self.casings.remove(&name.to_ascii_lowercase());
    }

    /// *Set*: the first field named `name` takes `value` and the others
    /// go; with none, the field is appended.
    pub fn set(&mut self, name: &[u8], value: &[u8]) {
        match self
            .fields
            .iter()
            .position(|(n, _)| n.eq_ignore_ascii_case(name))
        {
            Some(first) => {
                self.fields[first].1 = value.to_vec();
                let mut index = 0;
                self.fields.retain(|(n, _)| {
                    index += 1;
                    index - 1 == first || !n.eq_ignore_ascii_case(name)
                });
            }
            None => self.append(name, value),
        }
    }
}

/// The fields, each name and value isomorphic-decoded; the casings are
/// derived from them and left out.
impl fmt::Debug for HeaderList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(
                self.iter()
                    .map(|(name, value)| (isomorphic_decode(name), isomorphic_decode(value))),
            )
            .finish()
    }
}

/// HTTP whitespace: LF, CR, TAB and SPACE (not FF).
pub(crate) fn is_http_whitespace(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\t' | ' ')
}

/// An HTTP tab or space.
pub(crate) fn is_http_tab_or_space(c: char) -> bool {
    matches!(c, '\t' | ' ')
}

/// An HTTP token code point: what RFC 9110's `token` is made of.
pub(crate) fn is_http_token_code_point(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(
            c,
            '!' | '#'
                | '$'
                | '%'
                | '&'
                | '\''
                | '*'
                | '+'
                | '-'
                | '.'
                | '^'
                | '_'
                | '`'
                | '|'
                | '~'
        )
}

/// An HTTP quoted-string token code point: TAB, U+0020 to U+007E, and
/// U+0080 to U+00FF.
pub(crate) fn is_http_quoted_string_token_code_point(c: char) -> bool {
    matches!(c, '\t' | ' '..='~' | '\u{80}'..='\u{FF}')
}

/// Collects the code points from `position` on while `predicate` holds
/// (Infra's *collect a sequence of code points*).
pub(crate) fn collect(
    input: &[char],
    position: &mut usize,
    predicate: impl Fn(char) -> bool,
) -> String {
    let start = *position;
    while *position < input.len() && predicate(input[*position]) {
        *position += 1;
    }
    input[start..*position].iter().collect()
}

/// *Collect an HTTP quoted string* from `input` at `position`, which is
/// at a `"`. With `extract_value`, the string's value: what is between
/// the quotes, each backslash escape resolved. Without, the code points
/// it spans, quotes and backslashes included.
pub(crate) fn collect_http_quoted_string(
    input: &[char],
    position: &mut usize,
    extract_value: bool,
) -> String {
    let start = *position;
    let mut value = String::new();
    debug_assert_eq!(input.get(*position), Some(&'"'));
    *position += 1;
    loop {
        value.push_str(&collect(input, position, |c| c != '"' && c != '\\'));
        if *position >= input.len() {
            break;
        }
        let quote_or_backslash = input[*position];
        *position += 1;
        if quote_or_backslash == '\\' {
            match input.get(*position) {
                None => {
                    value.push('\\');
                    break;
                }
                Some(&c) => {
                    value.push(c);
                    *position += 1;
                }
            }
        } else {
            break;
        }
    }
    if extract_value {
        value
    } else {
        // The quoted string runs from its opening quote up to where
        // collecting stopped: past the closing quote, or at the end.
        input[start..*position].iter().collect()
    }
}

/// *Get, decode, and split* a header value: isomorphic-decoded, split at
/// each comma outside a quoted string, each piece without its leading
/// and trailing tabs and spaces. A value always gives at least one piece.
fn get_decode_split_value(value: &[u8]) -> Vec<String> {
    let input: Vec<char> = isomorphic_decode(value).chars().collect();
    let mut position = 0;
    let mut values = Vec::new();
    let mut temporary = String::new();
    loop {
        temporary.push_str(&collect(&input, &mut position, |c| c != '"' && c != ','));
        if position < input.len() && input[position] == '"' {
            temporary.push_str(&collect_http_quoted_string(&input, &mut position, false));
            if position < input.len() {
                continue;
            }
        }
        values.push(temporary.trim_matches(is_http_tab_or_space).to_owned());
        temporary.clear();
        if position >= input.len() {
            return values;
        }
        debug_assert_eq!(input[position], ',');
        position += 1;
    }
}

/// Whether `name` is a header name: a token (Fetch 2.2.2).
pub fn is_header_name(name: &[u8]) -> bool {
    !name.is_empty()
        && name
            .iter()
            .all(|&b| is_http_token_code_point(char::from(b)))
}

/// Whether `value` is a header value (Fetch 2.2.2): no HTTP tab or
/// space at either end, and no NUL, CR or LF.
pub fn is_header_value(value: &[u8]) -> bool {
    let edge = |b: &u8| matches!(b, b'\t' | b' ');
    !value.first().is_some_and(edge)
        && !value.last().is_some_and(edge)
        && !value.iter().any(|&b| matches!(b, b'\0' | b'\r' | b'\n'))
}

/// `value` normalized (Fetch 2.2.2): its leading and trailing HTTP
/// whitespace bytes removed.
pub fn normalize(value: &[u8]) -> &[u8] {
    let space = |b: &u8| matches!(b, b'\n' | b'\r' | b'\t' | b' ');
    let start = value.iter().position(|b| !space(b)).unwrap_or(value.len());
    let end = value
        .iter()
        .rposition(|b| !space(b))
        .map_or(start, |end| end + 1);
    &value[start..end]
}

/// Whether `method` is a forbidden method (Fetch 2.2.1): `CONNECT`,
/// `TRACE` or `TRACK`, in any case.
pub fn is_forbidden_method(method: &[u8]) -> bool {
    [b"CONNECT".as_slice(), b"TRACE", b"TRACK"]
        .iter()
        .any(|forbidden| method.eq_ignore_ascii_case(forbidden))
}

/// The request headers a script may not set (Fetch 2.2.2's forbidden
/// request-header names), lower case.
const FORBIDDEN_REQUEST_NAMES: [&[u8]; 21] = [
    b"accept-charset",
    b"accept-encoding",
    b"access-control-request-headers",
    b"access-control-request-method",
    b"connection",
    b"content-length",
    b"cookie",
    b"cookie2",
    b"date",
    b"dnt",
    b"expect",
    b"host",
    b"keep-alive",
    b"origin",
    b"referer",
    b"set-cookie",
    b"te",
    b"trailer",
    b"transfer-encoding",
    b"upgrade",
    b"via",
];

/// Whether (`name`, `value`) is a forbidden request-header (Fetch
/// 2.2.2): one of the names above, a name starting `proxy-` or `sec-`,
/// or a method override naming a forbidden method.
pub fn is_forbidden_request_header(name: &[u8], value: &[u8]) -> bool {
    let name = name.to_ascii_lowercase();
    if FORBIDDEN_REQUEST_NAMES.contains(&name.as_slice())
        || name.starts_with(b"proxy-")
        || name.starts_with(b"sec-")
    {
        return true;
    }
    matches!(
        name.as_slice(),
        b"x-http-method" | b"x-http-method-override" | b"x-method-override"
    ) && get_decode_split_value(value)
        .iter()
        .any(|method| is_forbidden_method(&isomorphic_encode(method)))
}

/// Whether `name` is a forbidden response-header name (Fetch 2.2.2): a
/// script never reads `Set-Cookie` or `Set-Cookie2`.
pub fn is_forbidden_response_header_name(name: &[u8]) -> bool {
    name.eq_ignore_ascii_case(b"set-cookie") || name.eq_ignore_ascii_case(b"set-cookie2")
}

/// Whether `name` is a request-body-header name (Fetch 2.2.2), which a
/// redirect that drops the body drops with it.
pub fn is_request_body_header_name(name: &[u8]) -> bool {
    [
        b"content-encoding".as_slice(),
        b"content-language",
        b"content-location",
        b"content-type",
    ]
    .iter()
    .any(|body| name.eq_ignore_ascii_case(body))
}

/// A CORS-unsafe request-header byte (Fetch 2.2.2).
fn is_cors_unsafe_byte(b: u8) -> bool {
    (b < 0x20 && b != b'\t')
        || matches!(
            b,
            b'"' | b'('
                | b')'
                | b':'
                | b'<'
                | b'>'
                | b'?'
                | b'@'
                | b'['
                | b'\\'
                | b']'
                | b'{'
                | b'}'
                | 0x7F
        )
}

/// Whether (`name`, `value`) is a CORS-safelisted request-header (Fetch
/// 2.2.2): one a form could send, which needs no preflight.
pub fn is_cors_safelisted_request_header(name: &[u8], value: &[u8]) -> bool {
    if value.len() > 128 {
        return false;
    }
    match name.to_ascii_lowercase().as_slice() {
        b"accept" => !value.iter().any(|&b| is_cors_unsafe_byte(b)),
        b"accept-language" | b"content-language" => value.iter().all(|&b| {
            b.is_ascii_alphanumeric() || matches!(b, b' ' | b'*' | b',' | b'-' | b'.' | b';' | b'=')
        }),
        b"content-type" => {
            !value.iter().any(|&b| is_cors_unsafe_byte(b))
                && crate::mime::MimeType::parse(&isomorphic_decode(value)).is_some_and(|mime| {
                    matches!(
                        mime.essence(),
                        "application/x-www-form-urlencoded" | "multipart/form-data" | "text/plain"
                    )
                })
        }
        b"range" => parse_single_range(value, false).is_some_and(|(start, _)| start.is_some()),
        _ => false,
    }
}

/// A single range header value (Fetch 2.2.2's "parse a single range
/// header value"), tabs and spaces between its tokens allowed when
/// `whitespace`: `bytes=` and a start, an end, or both; `None` when it
/// is none, or the start is past the end.
pub(crate) fn parse_single_range(
    value: &[u8],
    whitespace: bool,
) -> Option<(Option<u64>, Option<u64>)> {
    let skip = |rest: &mut &[u8]| {
        while whitespace && let [b' ' | b'\t', tail @ ..] = *rest {
            *rest = tail;
        }
    };
    let number = |rest: &mut &[u8]| -> Option<Option<u64>> {
        let count = rest.iter().take_while(|b| b.is_ascii_digit()).count();
        let (digits, tail) = rest.split_at(count);
        *rest = tail;
        if digits.is_empty() {
            return Some(None);
        }
        std::str::from_utf8(digits).ok()?.parse().ok().map(Some)
    };
    let mut rest = value
        .get(..5)
        .filter(|unit| unit.eq_ignore_ascii_case(b"bytes"))
        .map(|_| &value[5..])?;
    skip(&mut rest);
    rest = rest.strip_prefix(b"=")?;
    skip(&mut rest);
    let start = number(&mut rest)?;
    skip(&mut rest);
    rest = rest.strip_prefix(b"-")?;
    skip(&mut rest);
    let end = number(&mut rest)?;
    skip(&mut rest);
    if !rest.is_empty() {
        return None;
    }
    match (start, end) {
        (None, None) => None,
        (Some(start), Some(end)) if start > end => None,
        _ => Some((start, end)),
    }
}

/// The CORS-unsafe request-header names of `headers` (Fetch 2.2.2),
/// lower case, sorted and each once: those not safelisted, and every
/// safelisted one too when their values come to more than 1,024 bytes.
pub fn cors_unsafe_request_header_names(headers: &HeaderList) -> Vec<Vec<u8>> {
    let mut unsafe_names = Vec::new();
    let mut potentially_unsafe = Vec::new();
    let mut safelisted_size = 0;
    for (name, value) in headers.iter() {
        if is_cors_safelisted_request_header(name, value) {
            potentially_unsafe.push(name);
            safelisted_size += value.len();
        } else {
            unsafe_names.push(name);
        }
    }
    if safelisted_size > 1024 {
        unsafe_names.extend(potentially_unsafe);
    }
    sorted_lowercase(unsafe_names)
}

/// Header names as a sorted, lower-case set (Fetch's "convert header
/// names to a sorted-lowercase set").
fn sorted_lowercase<'a>(names: impl IntoIterator<Item = &'a [u8]>) -> Vec<Vec<u8>> {
    let mut names: Vec<Vec<u8>> = names.into_iter().map(<[u8]>::to_ascii_lowercase).collect();
    names.sort();
    names.dedup();
    names
}

/// Whether `name` is a no-CORS-safelisted request-header name (Fetch
/// 2.2.2), a name a `no-cors` request may carry.
pub fn is_no_cors_safelisted_request_header_name(name: &[u8]) -> bool {
    [
        b"accept".as_slice(),
        b"accept-language",
        b"content-language",
        b"content-type",
    ]
    .iter()
    .any(|safe| name.eq_ignore_ascii_case(safe))
}

/// Whether `name` is a CORS-safelisted response-header name given the
/// names a response exposes (Fetch 2.2.2): one a CORS response shows.
pub fn is_cors_safelisted_response_header_name(name: &[u8], exposed: &[Vec<u8>]) -> bool {
    [
        b"cache-control".as_slice(),
        b"content-language",
        b"content-length",
        b"content-type",
        b"expires",
        b"last-modified",
        b"pragma",
    ]
    .iter()
    .any(|safe| name.eq_ignore_ascii_case(safe))
        || (!is_forbidden_response_header_name(name)
            && exposed
                .iter()
                .any(|exposed| name.eq_ignore_ascii_case(exposed)))
}

/// The values of a header whose grammar is a comma-separated list of
/// tokens (`Access-Control-Expose-Headers`, `-Allow-Headers`,
/// `-Allow-Methods`; Fetch's "extract header list values"): `None` when
/// the list has none, `Some(Err(()))` when one is not a token. Empty
/// elements are skipped, as the list grammar allows.
pub fn token_list(headers: &HeaderList, name: &[u8]) -> Option<Result<Vec<Vec<u8>>, ()>> {
    let value = headers.get(name)?;
    let mut values = Vec::new();
    for item in value.split(|&b| b == b',') {
        let item = normalize(item);
        if item.is_empty() {
            continue;
        }
        if !is_header_name(item) {
            return Some(Err(()));
        }
        values.push(item.to_vec());
    }
    Some(Ok(values))
}

impl HeaderList {
    /// *Sort and combine* (Fetch 2.2.2): the names lower case and sorted,
    /// each name's values joined by `, `, but `set-cookie`'s, each a pair
    /// of its own; what `Headers` iterates by.
    pub fn sort_and_combine(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
        let mut out = Vec::new();
        for name in sorted_lowercase(self.iter().map(|(name, _)| name)) {
            if name == b"set-cookie" {
                for (_, value) in self.iter().filter(|(n, _)| n.eq_ignore_ascii_case(&name)) {
                    out.push((name.clone(), value.to_vec()));
                }
            } else {
                let value = self.get(&name).expect("a name of the list");
                out.push((name, value));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(fields: &[(&str, &str)]) -> HeaderList {
        let mut list = HeaderList::new();
        for (name, value) in fields {
            list.append(name.as_bytes(), value.as_bytes());
        }
        list
    }

    fn split(fields: &[(&str, &str)]) -> Option<Vec<String>> {
        list(fields).get_decode_split(b"A")
    }

    #[test]
    fn names_and_values_are_what_fetch_allows() {
        assert!(is_header_name(b"X-A") && is_header_name(b"*"));
        assert!(!is_header_name(b"") && !is_header_name(b"a b") && !is_header_name(b"a:"));
        assert!(is_header_value(b"") && is_header_value(b"a b"));
        for bad in [b" a".as_slice(), b"a\t", b"a\nb", b"a\rb", b"a\0"] {
            assert!(!is_header_value(bad), "{bad:?}");
        }
        assert_eq!(normalize(b" \t\r\na b\n "), b"a b");
        assert_eq!(normalize(b" \t"), b"");
    }

    #[test]
    fn forbidden_request_headers_and_methods() {
        for name in [
            "Cookie",
            "HOST",
            "origin",
            "Sec-Fetch-Site",
            "proxy-x",
            "Content-Length",
        ] {
            assert!(is_forbidden_request_header(name.as_bytes(), b"1"), "{name}");
        }
        for name in [
            "X-A",
            "Accept",
            "User-Agent",
            "Authorization",
            "sec",
            "proxy",
        ] {
            assert!(
                !is_forbidden_request_header(name.as_bytes(), b"1"),
                "{name}"
            );
        }
        assert!(is_forbidden_request_header(
            b"X-HTTP-Method-Override",
            b"GET, trace"
        ));
        assert!(is_forbidden_request_header(
            b"x-method-override",
            b"Connect"
        ));
        assert!(!is_forbidden_request_header(b"X-HTTP-Method", b"PUT"));
        assert!(is_forbidden_method(b"track") && !is_forbidden_method(b"PATCH"));
        assert!(is_forbidden_response_header_name(b"Set-Cookie2"));
        assert!(!is_forbidden_response_header_name(b"Cookie"));
        assert!(is_request_body_header_name(b"Content-Type"));
        assert!(!is_request_body_header_name(b"Content-Length"));
    }

    #[test]
    fn safelisted_request_headers_need_no_preflight() {
        let safe = |name: &str, value: &str| {
            is_cors_safelisted_request_header(name.as_bytes(), value.as_bytes())
        };
        assert!(safe("Accept", "x"));
        assert!(!safe("Accept", "a(b"));
        assert!(safe("accept-language", "en-GB, en;q=0.9"));
        assert!(!safe("Content-Language", "en_GB"));
        assert!(safe("Content-Type", "text/plain;charset=UTF-8"));
        assert!(safe("content-type", "multipart/form-data; boundary=x"));
        assert!(!safe("Content-Type", "application/json"));
        assert!(!safe("Content-Type", "text/plain; a=\"b\""));
        assert!(safe("Range", "bytes=0-"));
        assert!(safe("Range", "bytes=5-10"));
        assert!(!safe("Range", "bytes=-5"));
        assert!(!safe("Range", "bytes=10-5"));
        assert!(!safe("Range", "bytes = 0-"));
        // A blob's fetch allows tabs and spaces between the tokens.
        assert_eq!(
            parse_single_range(b"bytes 	= 1 - 2 ", true),
            Some((Some(1), Some(2)))
        );
        assert_eq!(parse_single_range(b"Bytes=-3", true), Some((None, Some(3))));
        assert_eq!(parse_single_range(b"bytes=1-2,3-4", true), None);
        assert_eq!(parse_single_range(b"bytes=-", true), None);
        assert!(!safe("X-A", "1"));
        assert!(!safe("Accept", &"a".repeat(129)));
        assert!(safe("Accept", &"a".repeat(128)));
        let mut headers = list(&[("X-B", "1"), ("Accept", "a"), ("x-a", "2"), ("X-B", "3")]);
        assert_eq!(
            cors_unsafe_request_header_names(&headers),
            [b"x-a".to_vec(), b"x-b".to_vec()]
        );
        // Safelisted values of more than 1,024 bytes together are unsafe.
        let long = "a".repeat(100);
        for _ in 0..11 {
            headers.append(b"Accept-Language", long.as_bytes());
        }
        assert_eq!(
            cors_unsafe_request_header_names(&headers),
            [
                b"accept".to_vec(),
                b"accept-language".to_vec(),
                b"x-a".to_vec(),
                b"x-b".to_vec()
            ]
        );
        assert!(is_no_cors_safelisted_request_header_name(b"Content-Type"));
        assert!(!is_no_cors_safelisted_request_header_name(b"Range"));
    }

    #[test]
    fn a_cors_response_shows_safelisted_and_exposed_names() {
        let exposed = [b"x-custom".to_vec(), b"set-cookie".to_vec()];
        assert!(is_cors_safelisted_response_header_name(
            b"Content-Type",
            &[]
        ));
        assert!(is_cors_safelisted_response_header_name(
            b"X-Custom",
            &exposed
        ));
        assert!(!is_cors_safelisted_response_header_name(
            b"X-Other", &exposed
        ));
        assert!(!is_cors_safelisted_response_header_name(
            b"Set-Cookie",
            &exposed
        ));
        let headers = list(&[("A", " x , ,y"), ("A", "z"), ("B", "a b")]);
        assert_eq!(
            token_list(&headers, b"a"),
            Some(Ok(vec![b"x".to_vec(), b"y".to_vec(), b"z".to_vec()]))
        );
        assert_eq!(token_list(&headers, b"b"), Some(Err(())));
        assert_eq!(token_list(&headers, b"c"), None);
    }

    #[test]
    fn sort_and_combine_lowercases_sorts_and_keeps_set_cookie_apart() {
        let headers = list(&[
            ("B", "1"),
            ("a", "2"),
            ("Set-Cookie", "x=1"),
            ("A", "3"),
            ("set-cookie", "y=2"),
        ]);
        let pairs: Vec<(String, String)> = headers
            .sort_and_combine()
            .into_iter()
            .map(|(n, v)| (String::from_utf8(n).unwrap(), String::from_utf8(v).unwrap()))
            .collect();
        let expected = [
            ("a", "2, 3"),
            ("b", "1"),
            ("set-cookie", "x=1"),
            ("set-cookie", "y=2"),
        ];
        assert_eq!(pairs, expected.map(|(n, v)| (n.to_owned(), v.to_owned())));
    }

    #[test]
    fn get_decode_split_as_fetch_shows_it() {
        // The examples in Fetch §2.2.2, in order.
        assert_eq!(split(&[("A", "nosniff,")]).unwrap(), ["nosniff", ""]);
        assert_eq!(split(&[("A", "")]).unwrap(), [""]);
        assert_eq!(split(&[("B", "sniff")]), None);
        assert_eq!(
            split(&[("A", "text/html;\", x/x")]).unwrap(),
            ["text/html;\", x/x"]
        );
        assert_eq!(
            split(&[("A", "x/x;test=\"hi\",y/y")]).unwrap(),
            ["x/x;test=\"hi\"", "y/y"]
        );
        assert_eq!(
            split(&[("A", "x / x,,,1")]).unwrap(),
            ["x / x", "", "", "1"]
        );
        assert_eq!(split(&[("A", "\"1,2\", 3")]).unwrap(), ["\"1,2\"", "3"]);
        // Several fields are one value joined by ", ".
        assert_eq!(
            split(&[("A", "nosniff"), ("B", "sniff"), ("A", "")]).unwrap(),
            ["nosniff", ""]
        );
        assert_eq!(
            split(&[("A", "x / x"), ("A", ","), ("A", "1")]).unwrap(),
            ["x / x", "", "", "1"]
        );
        assert_eq!(
            split(&[("A", "x/x;test=\"hi\""), ("C", "**bingo**"), ("A", "y/y")]).unwrap(),
            ["x/x;test=\"hi\"", "y/y"]
        );
        assert_eq!(
            split(&[("A", "text/html;\""), ("A", "x/x")]).unwrap(),
            ["text/html;\", x/x"]
        );
        assert_eq!(
            split(&[("A", "\"1,2\""), ("D", "4"), ("A", "3")]).unwrap(),
            ["\"1,2\"", "3"]
        );
    }

    #[test]
    fn values_are_isomorphic_decoded() {
        let mut headers = HeaderList::new();
        headers.append(b"A", b"caf\xE9, \xFF");
        assert_eq!(headers.get_decode_split(b"a").unwrap(), ["café", "ÿ"]);
    }

    #[test]
    fn quoted_strings_as_fetch_shows_them() {
        // The table in Fetch §2.2: input, output, with extract-value,
        // and where the position ends.
        let cases = [
            ("\"\\", "\"\\", "\\", 2),
            ("\"Hello\" World", "\"Hello\"", "Hello", 7),
            (
                "\"Hello \\\\ World\\\"\"",
                "\"Hello \\\\ World\\\"\"",
                "Hello \\ World\"",
                18,
            ),
        ];
        for (input, output, value, end) in cases {
            let input: Vec<char> = input.chars().collect();
            let mut position = 0;
            assert_eq!(
                collect_http_quoted_string(&input, &mut position, false),
                output
            );
            assert_eq!(position, end);
            let mut position = 0;
            assert_eq!(
                collect_http_quoted_string(&input, &mut position, true),
                value
            );
            assert_eq!(position, end);
        }
    }

    #[test]
    fn names_match_without_case_and_keep_their_first_casing() {
        let mut headers = list(&[("Content-Type", "a"), ("X-One", "1")]);
        headers.append(b"content-TYPE", b"b");
        assert!(headers.contains(b"CONTENT-type"));
        assert_eq!(headers.get(b"content-type").unwrap(), b"a, b");
        assert_eq!(headers.last(b"Content-Type").unwrap(), b"b");
        let names: Vec<&[u8]> = headers.iter().map(|(name, _)| name).collect();
        assert_eq!(names, [&b"Content-Type"[..], b"X-One", b"Content-Type"]);
    }

    #[test]
    fn casing_survives_delete_and_set() {
        let mut headers = list(&[("X-A", "1")]);
        headers.delete(b"x-a");
        headers.append(b"x-A", b"2");
        headers.set(b"X-a", b"3");
        headers.set(b"Y", b"4");
        headers.append(b"y", b"5");
        let names: Vec<&[u8]> = headers.iter().map(|(name, _)| name).collect();
        assert_eq!(names, [&b"x-A"[..], b"Y", b"Y"]);
        assert_eq!(headers, list(&[("x-A", "3"), ("Y", "4"), ("Y", "5")]));
    }

    #[test]
    fn set_keeps_the_first_place_and_drops_the_others() {
        let mut headers = list(&[("A", "1"), ("B", "2"), ("a", "3"), ("C", "4")]);
        headers.set(b"a", b"x");
        let fields: Vec<(&[u8], &[u8])> = headers.iter().collect();
        assert_eq!(fields, [(&b"A"[..], &b"x"[..]), (b"B", b"2"), (b"C", b"4")]);
        headers.set(b"D", b"5");
        assert_eq!(headers.last(b"d").unwrap(), b"5");
        headers.delete(b"b");
        assert!(!headers.contains(b"B"));
        assert_eq!(headers.len(), 3);
        assert!(!headers.is_empty());
    }

    #[test]
    fn code_point_classes() {
        let tokens: String = (' '..='~')
            .filter(|&c| is_http_token_code_point(c))
            .collect();
        assert_eq!(
            tokens,
            "!#$%&'*+-.0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ^_`abcdefghijklmnopqrstuvwxyz|~"
        );
        for c in ['\0', '\n', '\r', '\u{7F}', '\u{100}'] {
            assert!(!is_http_quoted_string_token_code_point(c), "{c:?}");
        }
        for c in ['\t', ' ', '"', '~', '\u{80}', '\u{FF}'] {
            assert!(is_http_quoted_string_token_code_point(c), "{c:?}");
        }
        assert!(!is_http_whitespace('\u{C}'));
    }
}
