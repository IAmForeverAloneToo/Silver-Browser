//! MIME types (MIME Sniffing §4): parsing, serializing, and the groups
//! the engine asks about; and a response's type as Fetch reads it from
//! its `Content-Type` fields (Fetch §3.5), with the encoding a `charset`
//! names and the `X-Content-Type-Options: nosniff` check (§3.6).
//! [`sniff`] finds a resource's computed type.

pub mod sniff;

use std::fmt;

use encoding::Encoding;

use crate::headers::{
    HeaderList, collect, collect_http_quoted_string, is_http_quoted_string_token_code_point,
    is_http_token_code_point, is_http_whitespace,
};
use crate::infra::{isomorphic_decode, isomorphic_encode};

/// A MIME type record: a type, a subtype, both lower-case tokens, and
/// parameters in the order they came, each name a lower-case token and
/// each value made of HTTP quoted-string token code points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MimeType {
    /// `type/subtype`.
    essence: String,
    /// Where the `/` is in `essence`.
    slash: usize,
    parameters: Vec<(String, String)>,
}

impl MimeType {
    /// *Parse a MIME type* (§4.4); `None` is failure.
    pub fn parse(input: &str) -> Option<MimeType> {
        let input: Vec<char> = input.chars().collect();
        let start = input
            .iter()
            .position(|&c| !is_http_whitespace(c))
            .unwrap_or(input.len());
        let end = input
            .iter()
            .rposition(|&c| !is_http_whitespace(c))
            .map_or(start, |last| last + 1);
        let input = &input[start..end];
        let mut position = 0;

        let type_name = collect(input, &mut position, |c| c != '/');
        if !is_token(&type_name) || position >= input.len() {
            return None;
        }
        position += 1;
        let subtype = collect(input, &mut position, |c| c != ';');
        let subtype = subtype.trim_end_matches(is_http_whitespace);
        if !is_token(subtype) {
            return None;
        }
        let mut mime_type = MimeType::new(
            &type_name.to_ascii_lowercase(),
            &subtype.to_ascii_lowercase(),
        );

        while position < input.len() {
            // Past the `;`, and the HTTP whitespace after it.
            position += 1;
            collect(input, &mut position, is_http_whitespace);
            let name = collect(input, &mut position, |c| c != ';' && c != '=').to_ascii_lowercase();
            if position < input.len() {
                if input[position] == ';' {
                    continue;
                }
                position += 1;
            }
            if position >= input.len() {
                break;
            }
            let value = if input[position] == '"' {
                let value = collect_http_quoted_string(input, &mut position, true);
                collect(input, &mut position, |c| c != ';');
                value
            } else {
                let value = collect(input, &mut position, |c| c != ';');
                let value = value.trim_end_matches(is_http_whitespace);
                if value.is_empty() {
                    continue;
                }
                value.to_owned()
            };
            if is_token(&name)
                && value.chars().all(is_http_quoted_string_token_code_point)
                && mime_type.parameter(&name).is_none()
            {
                mime_type.parameters.push((name, value));
            }
        }
        Some(mime_type)
    }

    /// *Parse a MIME type from bytes*: the bytes isomorphic-decoded, then
    /// parsed.
    pub fn parse_bytes(input: &[u8]) -> Option<MimeType> {
        MimeType::parse(&isomorphic_decode(input))
    }

    /// A type with no parameters, from a type and subtype already lower
    /// case and tokens.
    fn new(type_name: &str, subtype: &str) -> MimeType {
        debug_assert!(is_token(type_name) && is_token(subtype));
        MimeType {
            essence: format!("{type_name}/{subtype}"),
            slash: type_name.len(),
            parameters: Vec::new(),
        }
    }

    /// One of the engine's own types, written correctly.
    pub(crate) fn known(serialization: &str) -> MimeType {
        MimeType::parse(serialization).expect("a valid MIME type")
    }

    /// The type, before the `/`.
    pub fn type_name(&self) -> &str {
        &self.essence[..self.slash]
    }

    /// The subtype, after the `/`.
    pub fn subtype(&self) -> &str {
        &self.essence[self.slash + 1..]
    }

    /// The essence: `type/subtype`.
    pub fn essence(&self) -> &str {
        &self.essence
    }

    /// The value of the parameter `name`, which is lower case.
    pub fn parameter(&self, name: &str) -> Option<&str> {
        self.parameters
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, value)| value.as_str())
    }

    /// The parameters, in order, as `(name, value)`.
    pub fn parameters(&self) -> impl Iterator<Item = (&str, &str)> {
        self.parameters
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }

    /// Sets the parameter `name`, a lower-case token, to `value`: in its
    /// place if it is there, and last if not.
    pub fn set_parameter(&mut self, name: &str, value: &str) {
        debug_assert!(is_token(name) && name.bytes().all(|b| !b.is_ascii_uppercase()));
        debug_assert!(value.chars().all(is_http_quoted_string_token_code_point));
        match self.parameters.iter_mut().find(|(n, _)| n == name) {
            Some((_, v)) => *v = value.to_owned(),
            None => self.parameters.push((name.to_owned(), value.to_owned())),
        }
    }

    /// *Serialize a MIME type to bytes*: the serialization, isomorphic
    /// encoded, as a `Content-Type` value carries it.
    pub fn to_bytes(&self) -> Vec<u8> {
        isomorphic_encode(&self.to_string())
    }

    /// An HTML MIME type: `text/html`.
    pub fn is_html(&self) -> bool {
        self.essence == "text/html"
    }

    /// An XML MIME type: a subtype ending in `+xml`, or `text/xml` or
    /// `application/xml`.
    pub fn is_xml(&self) -> bool {
        self.subtype().ends_with("+xml") || matches!(self.essence(), "text/xml" | "application/xml")
    }

    /// A scriptable MIME type: XML, HTML, or `application/pdf`.
    pub fn is_scriptable(&self) -> bool {
        self.is_xml() || self.is_html() || self.essence == "application/pdf"
    }

    /// An image MIME type: the type `image`.
    pub fn is_image(&self) -> bool {
        self.type_name() == "image"
    }

    /// An audio or video MIME type: the type `audio` or `video`, or
    /// `application/ogg`.
    pub fn is_audio_or_video(&self) -> bool {
        matches!(self.type_name(), "audio" | "video") || self.essence == "application/ogg"
    }

    /// A JavaScript MIME type: one of the sixteen essences MIME Sniffing
    /// §4.6 lists.
    pub fn is_javascript(&self) -> bool {
        matches!(
            self.essence.as_str(),
            "application/ecmascript"
                | "application/javascript"
                | "application/x-ecmascript"
                | "application/x-javascript"
                | "text/ecmascript"
                | "text/javascript"
                | "text/javascript1.0"
                | "text/javascript1.1"
                | "text/javascript1.2"
                | "text/javascript1.3"
                | "text/javascript1.4"
                | "text/javascript1.5"
                | "text/jscript"
                | "text/livescript"
                | "text/x-ecmascript"
                | "text/x-javascript"
        )
    }

    /// A JSON MIME type: a subtype ending in `+json`, or `application/json`
    /// or `text/json` (MIME Sniffing §4.6).
    pub fn is_json(&self) -> bool {
        self.subtype().ends_with("+json")
            || matches!(self.essence.as_str(), "application/json" | "text/json")
    }
}

/// *Serialize a MIME type* (§4.5). A value that is not a non-empty token
/// is quoted, with each `"` and `\` escaped.
impl fmt::Display for MimeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.essence)?;
        for (name, value) in &self.parameters {
            write!(f, ";{name}=")?;
            if is_token(value) {
                f.write_str(value)?;
            } else {
                f.write_str("\"")?;
                for c in value.chars() {
                    if matches!(c, '"' | '\\') {
                        f.write_str("\\")?;
                    }
                    write!(f, "{c}")?;
                }
                f.write_str("\"")?;
            }
        }
        Ok(())
    }
}

/// A non-empty string of HTTP token code points.
fn is_token(s: &str) -> bool {
    !s.is_empty() && s.chars().all(is_http_token_code_point)
}

/// *Extract a MIME type* from a header list (Fetch §3.5): of the
/// `Content-Type` values, the last that parses and is not `*/*`. A
/// `charset` carries over to a later value of the same essence that has
/// none. `None` is failure.
pub fn extract_mime_type(headers: &HeaderList) -> Option<MimeType> {
    let values = headers.get_decode_split(b"Content-Type")?;
    let mut charset: Option<String> = None;
    let mut essence: Option<String> = None;
    let mut mime_type = None;
    for value in values {
        let Some(mut current) = MimeType::parse(&value) else {
            continue;
        };
        if current.essence() == "*/*" {
            continue;
        }
        if essence.as_deref() != Some(current.essence()) {
            charset = current.parameter("charset").map(str::to_owned);
            essence = Some(current.essence().to_owned());
        } else if current.parameter("charset").is_none()
            && let Some(charset) = &charset
        {
            current.set_parameter("charset", charset);
        }
        mime_type = Some(current);
    }
    mime_type
}

/// *Legacy extract an encoding* (Fetch §3.5): the encoding the type's
/// `charset` names, or `fallback` when there is no type, no `charset`,
/// or no encoding by that label.
pub fn legacy_extract_encoding(mime_type: Option<&MimeType>, fallback: Encoding) -> Encoding {
    mime_type
        .and_then(|mime_type| mime_type.parameter("charset"))
        .and_then(Encoding::for_label)
        .unwrap_or(fallback)
}

/// *Determine nosniff* (Fetch §3.6): whether the first value of
/// `X-Content-Type-Options` is `nosniff`, in any case.
pub fn determine_nosniff(headers: &HeaderList) -> bool {
    headers
        .get_decode_split(b"X-Content-Type-Options")
        .and_then(|values| values.into_iter().next())
        .is_some_and(|first| first.eq_ignore_ascii_case("nosniff"))
}

/// Whether a response of `mime_type`, with `headers`, may not be a
/// script: Fetch's "should response to request be blocked due to its MIME
/// type?" (an audio, image or video type, or `text/csv`) and "... due to
/// nosniff?" (under `X-Content-Type-Options: nosniff`, no type or one not
/// JavaScript), for a script-like destination.
pub fn blocked_as_script(mime_type: Option<&MimeType>, headers: &HeaderList) -> bool {
    if determine_nosniff(headers) && !mime_type.is_some_and(MimeType::is_javascript) {
        return true;
    }
    mime_type.is_some_and(|mime_type| {
        let essence = mime_type.essence();
        ["audio/", "image/", "video/"]
            .iter()
            .any(|kind| essence.starts_with(kind))
            || essence == "text/csv"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_may_have_any_type_but_media_and_csv_or_under_nosniff_javascript() {
        let blocked = |content_type: Option<&str>, nosniff: bool| {
            let mut headers = HeaderList::new();
            if let Some(value) = content_type {
                headers.append(b"Content-Type", value.as_bytes());
            }
            if nosniff {
                headers.append(b"X-Content-Type-Options", b"NoSniff");
            }
            blocked_as_script(extract_mime_type(&headers).as_ref(), &headers)
        };
        for allowed in [
            None,
            Some("text/plain"),
            Some("application/json"),
            Some("text/html"),
        ] {
            assert!(!blocked(allowed, false), "{allowed:?}");
        }
        for media in [
            "image/png",
            "audio/ogg",
            "video/mp4",
            "text/csv",
            "Text/CSV; a=b",
        ] {
            assert!(blocked(Some(media), false), "{media}");
        }
        assert!(!blocked(Some("text/javascript"), true));
        assert!(!blocked(
            Some("application/x-javascript;charset=utf-8"),
            true
        ));
        assert!(blocked(Some("text/plain"), true));
        assert!(blocked(None, true));
    }

    fn content_types(values: &[&str]) -> HeaderList {
        let mut headers = HeaderList::new();
        for value in values {
            headers.append(b"Content-Type", value.as_bytes());
        }
        headers
    }

    fn extracted(values: &[&str]) -> Option<String> {
        extract_mime_type(&content_types(values)).map(|m| m.to_string())
    }

    #[test]
    fn extraction_as_fetch_shows_it() {
        // The table in Fetch §3.5, in order.
        assert_eq!(
            extracted(&["text/plain;charset=gbk, text/html"]).unwrap(),
            "text/html"
        );
        assert_eq!(
            extracted(&["text/html;charset=gbk;a=b, text/html;x=y"]).unwrap(),
            "text/html;x=y;charset=gbk"
        );
        assert_eq!(
            extracted(&["text/html;charset=gbk;a=b", "text/html;x=y"]).unwrap(),
            "text/html;x=y;charset=gbk"
        );
        assert_eq!(
            extracted(&["text/html;charset=gbk", "x/x", "text/html;x=y"]).unwrap(),
            "text/html;x=y"
        );
        assert_eq!(
            extracted(&["text/html", "cannot-parse"]).unwrap(),
            "text/html"
        );
        assert_eq!(extracted(&["text/html", "*/*"]).unwrap(), "text/html");
        assert_eq!(extracted(&["text/html", ""]).unwrap(), "text/html");
        assert_eq!(extracted(&[]), None);
        assert_eq!(extracted(&["*/*"]), None);
        assert_eq!(extracted(&["nonsense"]), None);
    }

    #[test]
    fn a_charset_carries_over_only_to_a_value_without_one() {
        assert_eq!(
            extracted(&[
                "text/html;charset=gbk",
                "text/html;charset=utf-8",
                "text/html"
            ])
            .unwrap(),
            // The second value's charset stands for itself; the third
            // takes the first's, as Fetch keeps only the first one seen
            // for an essence.
            "text/html;charset=gbk"
        );
        assert_eq!(
            extracted(&["text/html;charset=gbk", "text/html;charset=utf-8"]).unwrap(),
            "text/html;charset=utf-8"
        );
    }

    #[test]
    fn the_legacy_encoding_comes_from_charset() {
        let html = |charset: &str| MimeType::parse(&format!("text/html;charset={charset}"));
        let fallback = encoding::UTF_8;
        assert_eq!(
            legacy_extract_encoding(html("latin1").as_ref(), fallback),
            encoding::WINDOWS_1252
        );
        assert_eq!(
            legacy_extract_encoding(html(" no-such-label").as_ref(), fallback),
            fallback
        );
        assert_eq!(
            legacy_extract_encoding(MimeType::parse("text/html").as_ref(), fallback),
            fallback
        );
        assert_eq!(legacy_extract_encoding(None, fallback), fallback);
    }

    #[test]
    fn nosniff_is_the_first_value_alone() {
        let headers = |values: &[&str]| {
            let mut headers = HeaderList::new();
            for value in values {
                headers.append(b"X-Content-Type-Options", value.as_bytes());
            }
            headers
        };
        assert!(determine_nosniff(&headers(&["nosniff"])));
        assert!(determine_nosniff(&headers(&["NoSniff, sniff"])));
        assert!(determine_nosniff(&headers(&[" nosniff\t"])));
        assert!(!determine_nosniff(&headers(&["sniff, nosniff"])));
        assert!(!determine_nosniff(&headers(&["", "nosniff"])));
        assert!(!determine_nosniff(&headers(&[])));
    }

    #[test]
    fn groups() {
        let t = |s: &str| MimeType::parse(s).expect(s);
        assert!(t("text/html;charset=utf-8").is_html());
        assert!(!t("application/xhtml+xml").is_html());
        for xml in [
            "text/xml",
            "application/xml",
            "image/svg+xml",
            "application/xhtml+xml",
        ] {
            assert!(t(xml).is_xml() && t(xml).is_scriptable(), "{xml}");
        }
        assert!(t("application/pdf").is_scriptable());
        assert!(!t("text/plain").is_scriptable());
        assert!(t("image/png").is_image());
        assert!(t("video/webm").is_audio_or_video());
        assert!(t("application/ogg").is_audio_or_video());
        assert!(!t("application/octet-stream").is_audio_or_video());
        for js in [
            "text/javascript",
            "application/x-ecmascript",
            "text/JavaScript1.5",
        ] {
            assert!(t(js).is_javascript(), "{js}");
        }
        assert!(!t("text/javascript2.0").is_javascript());
        assert!(!t("application/json").is_javascript());
        for json in ["application/json", "text/json", "application/ld+json"] {
            assert!(t(json).is_json(), "{json}");
        }
        assert!(!t("application/jsonx").is_json());
    }

    #[test]
    fn accessors_and_bytes() {
        let mut t = MimeType::parse("Text/HTML; Charset=\"é\\\"\"; a=b").unwrap();
        assert_eq!(t.type_name(), "text");
        assert_eq!(t.subtype(), "html");
        assert_eq!(t.essence(), "text/html");
        assert_eq!(t.parameter("charset"), Some("é\""));
        assert_eq!(t.to_string(), "text/html;charset=\"é\\\"\";a=b");
        assert_eq!(t.to_bytes(), b"text/html;charset=\"\xE9\\\"\";a=b");
        assert_eq!(MimeType::parse_bytes(&t.to_bytes()), Some(t.clone()));
        t.set_parameter("a", "c");
        t.set_parameter("z", "");
        let parameters: Vec<_> = t.parameters().collect();
        assert_eq!(parameters, [("charset", "é\""), ("a", "c"), ("z", "")]);
        assert_eq!(t.to_string(), "text/html;charset=\"é\\\"\";a=c;z=\"\"");
    }
}
