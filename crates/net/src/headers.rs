//! Header lists (Fetch §2.2.2) and the pieces of Fetch §2.2 that read
//! them and MIME types: HTTP whitespace, token code points, collecting an
//! HTTP quoted string, and getting, decoding and splitting a value.
//!
//! A header list holds what arrived. Whether a name is a token and a
//! value free of NUL and stray CR is the HTTP reader's to check (step 5
//! of `docs/design/net.md`'s order of work), before a field gets here.

use std::collections::HashMap;

use crate::infra::isomorphic_decode;

/// A header list: header fields in the order they came, each name and
/// value as bytes. Names match ASCII case-insensitively, and a name may
/// occur more than once.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
