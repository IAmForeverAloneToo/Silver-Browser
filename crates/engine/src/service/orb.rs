//! Opaque Response Blocking (M34, D31; the `annevk/orb` explainer): which
//! responses to a renderer's `no-cors` requests of another origin may
//! reach it with their bodies. Images, scripts, style sheets and media go
//! through; HTML, JSON, XML, documents and archives do not, so that a
//! renderer taken over cannot read another site's pages and data by
//! asking for them as an image. The explainer's fifteen steps, step 14
//! (a body that parses as JavaScript but not as JSON) done as the
//! reference browser's first version does it: by looking for what is not
//! script (an HTML or XML start, a JSON value, a JSON parser breaker), with
//! no parser of JavaScript in the browser.

use net::mime::sniff::{audio_or_video_type, image_type};
use net::mime::{determine_nosniff, extract_mime_type};
use net::{MimeType, Response};

/// How many bytes of a body the sniffing steps look at.
const SNIFFED: usize = 1024;

/// How deep a JSON value may nest before the recognizer gives up on it,
/// which then blocks it.
const JSON_DEPTH: usize = 512;

/// The types never sniffed: documents, archives and the like, blocked
/// whatever their bytes (the explainer's list).
const NEVER_SNIFFED: &[&str] = &[
    "application/gzip",
    "application/msexcel",
    "application/mspowerpoint",
    "application/msword",
    "application/msword-template",
    "application/pdf",
    "application/vnd.ces-quickpoint",
    "application/vnd.ces-quicksheet",
    "application/vnd.ces-quickword",
    "application/vnd.ms-excel",
    "application/vnd.ms-excel.sheet.macroenabled.12",
    "application/vnd.ms-powerpoint",
    "application/vnd.ms-powerpoint.presentation.macroenabled.12",
    "application/vnd.ms-word",
    "application/vnd.ms-word.document.12",
    "application/vnd.ms-word.document.macroenabled.12",
    "application/vnd.msword",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    "application/vnd.openxmlformats-officedocument.presentationml.template",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.template",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.template",
    "application/vnd.presentation-openxml",
    "application/vnd.presentation-openxmlm",
    "application/vnd.spreadsheet-openxml",
    "application/vnd.wordprocessing-openxml",
    "application/x-gzip",
    "application/x-protobuf",
    "application/x-protobuffer",
    "application/zip",
    "audio/mpegurl",
    "multipart/byteranges",
    "multipart/signed",
    "text/csv",
    "text/event-stream",
];

/// A JavaScript MIME type, `text/css` or `image/svg+xml`.
fn safelisted(mime: &MimeType) -> bool {
    mime.is_javascript() || matches!(mime.essence(), "text/css" | "image/svg+xml")
}

/// An HTML, JSON or XML MIME type.
fn blocklisted(mime: &MimeType) -> bool {
    mime.is_html() || mime.is_json() || mime.is_xml()
}

/// Whether a 206's `Content-Range` starts at the body's first byte.
fn from_the_start(response: &Response) -> bool {
    response.headers.get(b"Content-Range").is_some_and(|range| {
        let range = String::from_utf8_lossy(&range).to_ascii_lowercase();
        range.trim_start().starts_with("bytes 0-")
    })
}

/// Whether `response`, to a `no-cors` request of another origin, may
/// reach the renderer with its body. Its media request state is never
/// "initial" or "subsequent": this version plays no media.
pub(crate) fn allowed(response: &Response) -> bool {
    let mime = extract_mime_type(&response.headers);
    let nosniff = determine_nosniff(&response.headers);
    if let Some(mime) = &mime {
        if safelisted(mime) {
            return true;
        }
        if NEVER_SNIFFED.contains(&mime.essence()) {
            return false;
        }
        if blocklisted(mime) && (response.status == 206 || nosniff) {
            return false;
        }
        if nosniff && mime.essence() == "text/plain" {
            return false;
        }
    }
    if response.status == 206 && !from_the_start(response) {
        return false;
    }
    let head = &response.body[..response.body.len().min(SNIFFED)];
    // Audio and video go through only for a media request, which no
    // element makes yet.
    if audio_or_video_type(head).is_some() {
        return false;
    }
    if image_type(head).is_some() {
        return true;
    }
    if nosniff || !(200..300).contains(&response.status) {
        return false;
    }
    let Some(mime) = mime else {
        return true;
    };
    if ["audio/", "image/", "video/"]
        .iter()
        .any(|kind| mime.essence().starts_with(kind))
    {
        return false;
    }
    !not_script(&response.body)
}

/// Whether `body` is something other than a script: HTML or XML by its
/// start, a JSON parser breaker, or a whole JSON value.
fn not_script(body: &[u8]) -> bool {
    let start = trimmed(skip_bom(body));
    const HTML: &[&[u8]] = &[
        b"<!doctype html",
        b"<html",
        b"<head",
        b"<script",
        b"<iframe",
        b"<h1",
        b"<div",
        b"<font",
        b"<table",
        b"<a",
        b"<style",
        b"<title",
        b"<b",
        b"<body",
        b"<br",
        b"<p",
        b"<!--",
    ];
    let starts = |pattern: &[u8]| {
        start.len() >= pattern.len() && start[..pattern.len()].eq_ignore_ascii_case(pattern)
    };
    let tag_ends = |pattern: &[u8]| {
        // `<a` and `<b` must end their name: `<abbr` is no sign.
        start
            .get(pattern.len())
            .is_none_or(|&next| next == b'>' || next.is_ascii_whitespace() || next == b'/')
    };
    if HTML
        .iter()
        .any(|pattern| starts(pattern) && (pattern.ends_with(b"--") || tag_ends(pattern)))
    {
        return true;
    }
    if starts(b"<?xml") {
        return true;
    }
    const BREAKERS: &[&[u8]] = &[b")]}'", b"{}&&", b"for(;;);"];
    if BREAKERS.iter().any(|breaker| start.starts_with(breaker)) {
        return true;
    }
    is_json(body)
}

fn skip_bom(body: &[u8]) -> &[u8] {
    body.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(body)
}

fn trimmed(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0C))
        .unwrap_or(bytes.len());
    &bytes[start..]
}

/// Whether `body` is one JSON value (RFC 8259), whitespace around it.
pub(crate) fn is_json(body: &[u8]) -> bool {
    let mut json = Json {
        bytes: skip_bom(body),
        at: 0,
    };
    json.space();
    json.value(0) && {
        json.space();
        json.at == json.bytes.len()
    }
}

struct Json<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Json<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn eat(&mut self, byte: u8) -> bool {
        let ate = self.peek() == Some(byte);
        self.at += usize::from(ate);
        ate
    }

    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn word(&mut self, word: &[u8]) -> bool {
        let ate = self.bytes[self.at..].starts_with(word);
        self.at += if ate { word.len() } else { 0 };
        ate
    }

    fn value(&mut self, depth: usize) -> bool {
        if depth > JSON_DEPTH {
            return false;
        }
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string(),
            Some(b't') => self.word(b"true"),
            Some(b'f') => self.word(b"false"),
            Some(b'n') => self.word(b"null"),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => false,
        }
    }

    fn object(&mut self, depth: usize) -> bool {
        self.at += 1;
        self.space();
        if self.eat(b'}') {
            return true;
        }
        loop {
            self.space();
            if !self.string() {
                return false;
            }
            self.space();
            if !self.eat(b':') {
                return false;
            }
            self.space();
            if !self.value(depth + 1) {
                return false;
            }
            self.space();
            if self.eat(b'}') {
                return true;
            }
            if !self.eat(b',') {
                return false;
            }
        }
    }

    fn array(&mut self, depth: usize) -> bool {
        self.at += 1;
        self.space();
        if self.eat(b']') {
            return true;
        }
        loop {
            self.space();
            if !self.value(depth + 1) {
                return false;
            }
            self.space();
            if self.eat(b']') {
                return true;
            }
            if !self.eat(b',') {
                return false;
            }
        }
    }

    fn string(&mut self) -> bool {
        if !self.eat(b'"') {
            return false;
        }
        while let Some(byte) = self.peek() {
            self.at += 1;
            match byte {
                b'"' => return true,
                b'\\' => match self.peek() {
                    Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => self.at += 1,
                    Some(b'u') => {
                        let hex = self.bytes.get(self.at + 1..self.at + 5);
                        if !hex.is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit)) {
                            return false;
                        }
                        self.at += 5;
                    }
                    _ => return false,
                },
                0..=0x1F => return false,
                _ => {}
            }
        }
        false
    }

    fn digits(&mut self) -> bool {
        let start = self.at;
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.at += 1;
        }
        self.at > start
    }

    fn number(&mut self) -> bool {
        self.eat(b'-');
        if !self.eat(b'0') && !self.digits() {
            return false;
        }
        if self.eat(b'.') && !self.digits() {
            return false;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.at += 1;
            if !self.eat(b'+') {
                self.eat(b'-');
            }
            if !self.digits() {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(status: u16, fields: &[(&str, &str)], body: &[u8]) -> Response {
        let mut headers = net::HeaderList::new();
        for (name, value) in fields {
            headers.append(name.as_bytes(), value.as_bytes());
        }
        Response {
            url_list: vec![net::Url::parse("https://other.test/r").expect("a URL")],
            status,
            status_message: Vec::new(),
            headers,
            body: body.to_vec(),
            mime_type: None,
            address_space: None,
            response_type: net::ResponseType::Opaque,
            cors_exposed: Vec::new(),
        }
    }

    fn typed(essence: &str, body: &[u8]) -> bool {
        allowed(&response(200, &[("Content-Type", essence)], body))
    }

    #[test]
    fn scripts_styles_and_images_go_through() {
        assert!(typed("text/javascript", b"<html> whatever"));
        assert!(typed("application/javascript", b"{\"a\": 1}"));
        assert!(typed("text/css", b"p {}"));
        assert!(typed("image/svg+xml", b"<svg/>"));
        assert!(typed("image/png", b"\x89PNG\r\n\x1a\n...."));
        assert!(typed("application/octet-stream", b"GIF89a...."), "sniffed");
        assert!(typed("text/plain", b"var x = [1, 2] + y;"), "a script");
        assert!(allowed(&response(200, &[], b"function f() {}")), "no type");
    }

    #[test]
    fn documents_data_and_archives_are_blocked() {
        assert!(!typed("text/html", b"<!DOCTYPE html><p>secret"));
        assert!(!typed("text/plain", b"  <html><body>secret"));
        assert!(!typed("application/json", b"{\"secret\": [1, 2]}"));
        assert!(!typed("text/plain", b"[1, 2, 3]"), "JSON is no script");
        assert!(!typed("text/plain", b"\"a string\""));
        assert!(
            !typed("text/plain", b")]}'\n{\"a\": 1}"),
            "a parser breaker"
        );
        assert!(!typed("text/plain", b"for(;;);{\"a\": 1}"));
        assert!(!typed("text/xml", b"<?xml version=\"1.0\"?><a/>"));
        assert!(!typed("application/pdf", b"%PDF-1.7"));
        assert!(!typed("application/zip", b"PK\x03\x04"));
        assert!(!typed("text/csv", b"a,b\n1,2"));
        assert!(
            !typed("image/png", b"not a picture"),
            "a type its bytes deny"
        );
        assert!(!typed("audio/mpeg", b"ID3\x03\x00\x00\x00"));
    }

    #[test]
    fn nosniff_errors_and_ranges_decide_first() {
        let nosniff = |essence: &str, body: &[u8]| {
            allowed(&response(
                200,
                &[
                    ("Content-Type", essence),
                    ("X-Content-Type-Options", "nosniff"),
                ],
                body,
            ))
        };
        assert!(!nosniff("text/html", b"var x;"));
        assert!(!nosniff("text/plain", b"var x;"));
        assert!(
            !nosniff("text/plain", b"\x89PNG\r\n\x1a\n...."),
            "plain text under nosniff is no image"
        );
        assert!(nosniff("text/javascript", b"var x;"));
        assert!(!nosniff("application/octet-stream", b"var x;"));
        assert!(!allowed(&response(
            404,
            &[("Content-Type", "text/plain")],
            b"var x;"
        )));
        let range = |from: &str| {
            allowed(&response(
                206,
                &[("Content-Type", "image/png"), ("Content-Range", from)],
                b"\x89PNG\r\n\x1a\n....",
            ))
        };
        assert!(range("bytes 0-11/100"));
        assert!(!range("bytes 50-61/100"));
        assert!(!allowed(&response(
            206,
            &[
                ("Content-Type", "text/html"),
                ("Content-Range", "bytes 0-9/9")
            ],
            b"<p>secret",
        )));
    }

    #[test]
    fn the_json_recognizer_takes_json_and_nothing_else() {
        for json in [
            &b"{}"[..],
            b" [ ] ",
            b"{\"a\": [1, -2.5e+3, true, false, null, \"\\u00e9\\n\"]}",
            b"0",
            b"\xEF\xBB\xBF{\"a\":1}",
        ] {
            assert!(is_json(json), "{}", String::from_utf8_lossy(json));
        }
        for not in [
            &b""[..],
            b"{a: 1}",
            b"[1,]",
            b"01",
            b"{\"a\" 1}",
            b"\"\\x\"",
            b"var a = {};",
            b"[1] + 1",
            b"\"open",
        ] {
            assert!(!is_json(not), "{}", String::from_utf8_lossy(not));
        }
        let deep = format!(
            "{}{}",
            "[".repeat(JSON_DEPTH + 2),
            "]".repeat(JSON_DEPTH + 2)
        );
        assert!(!is_json(deep.as_bytes()), "too deep to be taken for JSON");
        assert!(!not_script(deep.as_bytes()), "nor blocked as JSON");
    }
}
