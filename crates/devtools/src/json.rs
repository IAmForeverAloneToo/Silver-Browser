//! JSON (RFC 8259): a value, its parser and its writer, what the
//! DevTools protocol's messages are written in (M36, `docs/design/
//! devtools.md`). The parser is strict and bounded: a text past
//! [`MAX_MESSAGE_BYTES`], or nested past [`MAX_JSON_DEPTH`], is refused,
//! and so is anything RFC 8259 does not allow (a trailing comma, a
//! comment, a leading zero, a control character in a string). An
//! escaped surrogate that has no partner is read as U+FFFD, as a lone
//! surrogate cannot be a Rust string's.

use std::fmt;

/// The longest message the protocol reads or writes, in bytes: half the
/// longest message between processes (`engine::ipc::MAX_MESSAGE`), so
/// that one always fits in one.
pub const MAX_MESSAGE_BYTES: usize = 8 << 20;

/// How deep arrays and objects may nest in a message.
pub const MAX_JSON_DEPTH: usize = 64;

/// A JSON value. An object keeps its members in the order they came,
/// a name more than once among them as it came; [`Json::get`] finds
/// the last, as `JSON.parse` keeps it.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    /// Always finite: what is not is written as `null`.
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

/// Why a text is not a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonError {
    /// Longer than [`MAX_MESSAGE_BYTES`].
    TooLong,
    /// Nested deeper than [`MAX_JSON_DEPTH`].
    TooDeep,
    /// Not JSON at this byte.
    Invalid(usize),
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JsonError::TooLong => write!(f, "longer than {MAX_MESSAGE_BYTES} bytes"),
            JsonError::TooDeep => write!(f, "nested deeper than {MAX_JSON_DEPTH}"),
            JsonError::Invalid(at) => write!(f, "not JSON at byte {at}"),
        }
    }
}

impl std::error::Error for JsonError {}

impl Json {
    /// Reads `text`, the whole of it one value with whitespace around.
    pub fn parse(text: &str) -> Result<Json, JsonError> {
        if text.len() > MAX_MESSAGE_BYTES {
            return Err(JsonError::TooLong);
        }
        let mut parser = Parser {
            bytes: text.as_bytes(),
            at: 0,
        };
        parser.skip_ws();
        let value = parser.value(0)?;
        parser.skip_ws();
        if parser.at != parser.bytes.len() {
            return Err(JsonError::Invalid(parser.at));
        }
        Ok(value)
    }

    /// The value written as JSON, with no whitespace.
    pub fn write(&self) -> String {
        let mut out = String::new();
        self.write_into(&mut out);
        out
    }

    fn write_into(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(true) => out.push_str("true"),
            Json::Bool(false) => out.push_str("false"),
            Json::Number(n) => write_number(*n, out),
            Json::String(s) => write_string(s, out),
            Json::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write_into(out);
                }
                out.push(']');
            }
            Json::Object(members) => {
                out.push('{');
                for (i, (name, value)) in members.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_string(name, out);
                    out.push(':');
                    value.write_into(out);
                }
                out.push('}');
            }
        }
    }

    /// An object of `members`.
    pub fn object<'a>(members: impl IntoIterator<Item = (&'a str, Json)>) -> Json {
        Json::Object(
            members
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect(),
        )
    }

    /// The member `name` of an object, the last of that name; `None` for
    /// another value or a name it has not.
    pub fn get(&self, name: &str) -> Option<&Json> {
        match self {
            Json::Object(members) => members
                .iter()
                .rev()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// A number that is a whole one from 0 to 2^53, as `u64`.
    pub fn as_u64(&self) -> Option<u64> {
        let n = self.as_f64()?;
        ((0.0..=9_007_199_254_740_992.0).contains(&n) && n.fract() == 0.0).then_some(n as u64)
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(items) => Some(items),
            _ => None,
        }
    }
}

impl From<&str> for Json {
    fn from(s: &str) -> Json {
        Json::String(s.to_owned())
    }
}

impl From<String> for Json {
    fn from(s: String) -> Json {
        Json::String(s)
    }
}

impl From<bool> for Json {
    fn from(b: bool) -> Json {
        Json::Bool(b)
    }
}

impl From<f64> for Json {
    fn from(n: f64) -> Json {
        if n.is_finite() {
            Json::Number(n)
        } else {
            Json::Null
        }
    }
}

impl From<u64> for Json {
    fn from(n: u64) -> Json {
        Json::Number(n as f64)
    }
}

impl From<u32> for Json {
    fn from(n: u32) -> Json {
        Json::Number(f64::from(n))
    }
}

impl From<i64> for Json {
    fn from(n: i64) -> Json {
        Json::Number(n as f64)
    }
}

impl<T: Into<Json>> From<Option<T>> for Json {
    fn from(value: Option<T>) -> Json {
        value.map_or(Json::Null, Into::into)
    }
}

impl<T: Into<Json>> From<Vec<T>> for Json {
    fn from(items: Vec<T>) -> Json {
        Json::Array(items.into_iter().map(Into::into).collect())
    }
}

/// A number as JSON: a whole one under 2^53 without a fraction, any
/// other finite one as Rust's shortest form that reads back the same,
/// what is not finite as `null`.
fn write_number(n: f64, out: &mut String) {
    use std::fmt::Write;
    if !n.is_finite() {
        out.push_str("null");
    } else if n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
        let _ = write!(out, "{}", n as i64);
    } else {
        let _ = write!(out, "{n:?}");
    }
}

fn write_string(s: &str, out: &mut String) {
    use std::fmt::Write;
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // The line and paragraph separators too, so that a message
            // is safe inside a script's source as well.
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn skip_ws(&mut self) {
        while matches!(self.bytes.get(self.at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn invalid<T>(&self) -> Result<T, JsonError> {
        Err(JsonError::Invalid(self.at))
    }

    fn literal(&mut self, word: &[u8], value: Json) -> Result<Json, JsonError> {
        if self.bytes[self.at..].starts_with(word) {
            self.at += word.len();
            Ok(value)
        } else {
            self.invalid()
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, JsonError> {
        match self.bytes.get(self.at) {
            Some(b'n') => self.literal(b"null", Json::Null),
            Some(b't') => self.literal(b"true", Json::Bool(true)),
            Some(b'f') => self.literal(b"false", Json::Bool(false)),
            Some(b'"') => self.string().map(Json::String),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(b'[') => {
                if depth >= MAX_JSON_DEPTH {
                    return Err(JsonError::TooDeep);
                }
                self.at += 1;
                let mut items = Vec::new();
                self.skip_ws();
                if self.bytes.get(self.at) == Some(&b']') {
                    self.at += 1;
                    return Ok(Json::Array(items));
                }
                loop {
                    self.skip_ws();
                    items.push(self.value(depth + 1)?);
                    self.skip_ws();
                    match self.bytes.get(self.at) {
                        Some(b',') => self.at += 1,
                        Some(b']') => {
                            self.at += 1;
                            return Ok(Json::Array(items));
                        }
                        _ => return self.invalid(),
                    }
                }
            }
            Some(b'{') => {
                if depth >= MAX_JSON_DEPTH {
                    return Err(JsonError::TooDeep);
                }
                self.at += 1;
                let mut members = Vec::new();
                self.skip_ws();
                if self.bytes.get(self.at) == Some(&b'}') {
                    self.at += 1;
                    return Ok(Json::Object(members));
                }
                loop {
                    self.skip_ws();
                    if self.bytes.get(self.at) != Some(&b'"') {
                        return self.invalid();
                    }
                    let name = self.string()?;
                    self.skip_ws();
                    if self.bytes.get(self.at) != Some(&b':') {
                        return self.invalid();
                    }
                    self.at += 1;
                    self.skip_ws();
                    let value = self.value(depth + 1)?;
                    members.push((name, value));
                    self.skip_ws();
                    match self.bytes.get(self.at) {
                        Some(b',') => self.at += 1,
                        Some(b'}') => {
                            self.at += 1;
                            return Ok(Json::Object(members));
                        }
                        _ => return self.invalid(),
                    }
                }
            }
            _ => self.invalid(),
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.at;
        while matches!(self.bytes.get(self.at), Some(b'0'..=b'9')) {
            self.at += 1;
        }
        self.at - start
    }

    fn number(&mut self) -> Result<Json, JsonError> {
        let start = self.at;
        if self.bytes.get(self.at) == Some(&b'-') {
            self.at += 1;
        }
        match self.bytes.get(self.at) {
            Some(b'0') => self.at += 1,
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return self.invalid(),
        }
        if self.bytes.get(self.at) == Some(&b'.') {
            self.at += 1;
            if self.digits() == 0 {
                return self.invalid();
            }
        }
        if matches!(self.bytes.get(self.at), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.bytes.get(self.at), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if self.digits() == 0 {
                return self.invalid();
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.at])
            .map_err(|_| JsonError::Invalid(start))?;
        let n: f64 = text.parse().map_err(|_| JsonError::Invalid(start))?;
        // A number too large for a double is no number the protocol
        // has: taken as the largest, as JSON.parse takes it to Infinity.
        Ok(Json::Number(if n.is_finite() {
            n
        } else {
            n.signum() * f64::MAX
        }))
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        let Some(digits) = self.bytes.get(self.at..self.at + 4) else {
            return self.invalid();
        };
        let mut value = 0;
        for &d in digits {
            let v = (d as char)
                .to_digit(16)
                .ok_or(JsonError::Invalid(self.at))?;
            value = value * 16 + v;
        }
        self.at += 4;
        Ok(value)
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.at += 1; // the opening quote
        let mut out = String::new();
        loop {
            let start = self.at;
            while let Some(&b) = self.bytes.get(self.at) {
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                self.at += 1;
            }
            // The input is a &str: whole characters between ASCII bytes.
            out.push_str(
                std::str::from_utf8(&self.bytes[start..self.at])
                    .map_err(|_| JsonError::Invalid(start))?,
            );
            match self.bytes.get(self.at) {
                Some(b'"') => {
                    self.at += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.at += 1;
                    let Some(&escape) = self.bytes.get(self.at) else {
                        return self.invalid();
                    };
                    self.at += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let unit = self.hex4()?;
                            let c = if (0xD800..0xDC00).contains(&unit)
                                && self.bytes[self.at..].starts_with(b"\\u")
                            {
                                let back = self.at;
                                self.at += 2;
                                let low = self.hex4()?;
                                if (0xDC00..0xE000).contains(&low) {
                                    char::from_u32(
                                        0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00),
                                    )
                                } else {
                                    // Not its partner: read on its own.
                                    self.at = back;
                                    None
                                }
                            } else {
                                char::from_u32(unit)
                            };
                            out.push(c.unwrap_or('\u{FFFD}'));
                        }
                        _ => {
                            self.at -= 1;
                            return self.invalid();
                        }
                    }
                }
                _ => return self.invalid(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round(text: &str) -> String {
        Json::parse(text).expect("parses").write()
    }

    #[test]
    fn values_are_read_and_written_back() {
        assert_eq!(
            round(r#" {"a": [1, -2.5, 3e2, true, null, "x"]} "#),
            r#"{"a":[1,-2.5,300,true,null,"x"]}"#
        );
        assert_eq!(round("[]"), "[]");
        assert_eq!(round("{}"), "{}");
        assert_eq!(round("0.1"), "0.1");
        assert_eq!(round("-0"), "0");
        assert_eq!(round("1e400"), format!("{:?}", f64::MAX));
    }

    #[test]
    fn strings_are_escaped_and_unescaped() {
        let parsed = Json::parse(r#""a\"b\\c\/d\n\u00e9\ud83d\ude00\ud800x""#).expect("parses");
        assert_eq!(parsed.as_str(), Some("a\"b\\c/d\n\u{e9}\u{1F600}\u{FFFD}x"));
        assert_eq!(
            Json::from("q\"\\\n\u{1}\u{2028}").write(),
            r#""q\"\\\n\u0001\u2028""#
        );
        // What is written reads back the same.
        let text = "tab\t, nul\u{0}, emoji \u{1F600}";
        assert_eq!(
            Json::parse(&Json::from(text).write()).unwrap().as_str(),
            Some(text)
        );
    }

    #[test]
    fn what_rfc_8259_refuses_is_refused() {
        for text in [
            "",
            " ",
            "[1,]",
            "{\"a\":1,}",
            "01",
            "1.",
            ".5",
            "+1",
            "1e",
            "--1",
            "[1 2]",
            "{a:1}",
            "'a'",
            "\"a",
            "\"\\x\"",
            "\"\u{1}\"",
            "nul",
            "tru",
            "[",
            "{\"a\"}",
            "1 2",
            "\"\\u12\"",
            "/* */1",
            "NaN",
            "Infinity",
        ] {
            assert!(Json::parse(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn a_message_is_bounded_in_length_and_depth() {
        let deep = "[".repeat(MAX_JSON_DEPTH) + &"]".repeat(MAX_JSON_DEPTH);
        assert!(Json::parse(&deep).is_ok());
        let deeper = "[".repeat(MAX_JSON_DEPTH + 1) + &"]".repeat(MAX_JSON_DEPTH + 1);
        assert_eq!(Json::parse(&deeper), Err(JsonError::TooDeep));
        let long = format!("\"{}\"", "a".repeat(MAX_MESSAGE_BYTES));
        assert_eq!(Json::parse(&long), Err(JsonError::TooLong));
    }

    #[test]
    fn members_are_kept_in_order_and_the_last_of_a_name_is_found() {
        let value = Json::parse(r#"{"b":1,"a":2,"b":3}"#).expect("parses");
        assert_eq!(value.write(), r#"{"b":1,"a":2,"b":3}"#);
        assert_eq!(value.get("b").and_then(Json::as_u64), Some(3));
        assert_eq!(value.get("c"), None);
        assert_eq!(Json::Number(2.5).as_u64(), None);
        assert_eq!(Json::Number(-1.0).as_u64(), None);
        assert_eq!(Json::from(f64::NAN), Json::Null);
    }
}
