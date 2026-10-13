//! html5lib's tokenizer tests (`tests/html5lib/tokenizer/`, copied with
//! their licence from html5lib-tests at the commit its README names):
//! the tokenizer's conformance suite (M24.8). Every test runs from each
//! of its initial states twice: given its input at once, and given it a
//! character at a time with the tokens taken after each, which must come
//! to the same. Parse errors are not reported, so a test's `errors` are
//! not compared; `xmlViolation.test` is for the infoset coercion, which
//! is not this tokenizer's, and is not run.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use html::{Doctype, TextState, Token, Tokenizer};

/// A JSON value, read by [`Json::parse`]: what the test files need.
#[derive(Debug, Clone, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    fn parse(text: &str) -> Json {
        let chars: Vec<char> = text.chars().collect();
        let mut at = 0;
        let value = parse_value(&chars, &mut at);
        skip_space(&chars, &mut at);
        assert_eq!(at, chars.len(), "trailing text");
        value
    }

    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    fn str(&self) -> &str {
        match self {
            Json::String(s) => s,
            other => panic!("not a string: {other:?}"),
        }
    }

    fn array(&self) -> &[Json] {
        match self {
            Json::Array(items) => items,
            other => panic!("not an array: {other:?}"),
        }
    }
}

fn skip_space(chars: &[char], at: &mut usize) {
    while chars.get(*at).is_some_and(|c| c.is_whitespace()) {
        *at += 1;
    }
}

fn parse_value(chars: &[char], at: &mut usize) -> Json {
    skip_space(chars, at);
    match chars[*at] {
        '{' => {
            *at += 1;
            let mut members = Vec::new();
            loop {
                skip_space(chars, at);
                if chars[*at] == '}' {
                    *at += 1;
                    return Json::Object(members);
                }
                let Json::String(key) = parse_value(chars, at) else {
                    panic!("a key that is not a string");
                };
                skip_space(chars, at);
                assert_eq!(chars[*at], ':');
                *at += 1;
                members.push((key, parse_value(chars, at)));
                skip_space(chars, at);
                if chars[*at] == ',' {
                    *at += 1;
                }
            }
        }
        '[' => {
            *at += 1;
            let mut items = Vec::new();
            loop {
                skip_space(chars, at);
                if chars[*at] == ']' {
                    *at += 1;
                    return Json::Array(items);
                }
                items.push(parse_value(chars, at));
                skip_space(chars, at);
                if chars[*at] == ',' {
                    *at += 1;
                }
            }
        }
        '"' => {
            *at += 1;
            let mut out = String::new();
            // UTF-16 units, so that a surrogate pair of escapes joins; a
            // lone surrogate becomes U+FFFD (no test's input has one but
            // in the double-escaped tests, which are read below).
            let mut units: Vec<u16> = Vec::new();
            let flush = |units: &mut Vec<u16>, out: &mut String| {
                out.extend(char::decode_utf16(units.drain(..)).map(|r| r.unwrap_or('\u{fffd}')));
            };
            loop {
                let c = chars[*at];
                *at += 1;
                match c {
                    '"' => {
                        flush(&mut units, &mut out);
                        return Json::String(out);
                    }
                    '\\' => {
                        let e = chars[*at];
                        *at += 1;
                        let simple = match e {
                            '"' => Some('"'),
                            '\\' => Some('\\'),
                            '/' => Some('/'),
                            'b' => Some('\u{8}'),
                            'f' => Some('\u{c}'),
                            'n' => Some('\n'),
                            'r' => Some('\r'),
                            't' => Some('\t'),
                            'u' => None,
                            other => panic!("bad escape {other}"),
                        };
                        match simple {
                            Some(c) => {
                                flush(&mut units, &mut out);
                                out.push(c);
                            }
                            None => {
                                let hex: String = chars[*at..*at + 4].iter().collect();
                                *at += 4;
                                units.push(u16::from_str_radix(&hex, 16).expect("hex"));
                            }
                        }
                    }
                    c => {
                        flush(&mut units, &mut out);
                        out.push(c);
                    }
                }
            }
        }
        't' => {
            *at += 4;
            Json::Bool(true)
        }
        'f' => {
            *at += 5;
            Json::Bool(false)
        }
        'n' => {
            *at += 4;
            Json::Null
        }
        _ => {
            let start = *at;
            while chars
                .get(*at)
                .is_some_and(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E'))
            {
                *at += 1;
            }
            let number: String = chars[start..*at].iter().collect();
            Json::Number(number.parse().expect("a number"))
        }
    }
}

/// A double-escaped test's string unescaped once more: each `\uHHHH`
/// its code point; `None` if one is a lone surrogate, which a Rust
/// string cannot hold.
fn unescape(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut units = Vec::new();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && chars.get(i + 1) == Some(&'u') && i + 6 <= chars.len() {
            let hex: String = chars[i + 2..i + 6].iter().collect();
            if let Ok(unit) = u16::from_str_radix(&hex, 16) {
                units.push(unit);
                i += 6;
                continue;
            }
        }
        for c in char::decode_utf16(units.drain(..)) {
            out.push(c.ok()?);
        }
        out.push(chars[i]);
        i += 1;
    }
    for c in char::decode_utf16(units.drain(..)) {
        out.push(c.ok()?);
    }
    Some(out)
}

/// A token as the tests write it, attributes sorted, so that the two
/// compare.
#[derive(Debug, Clone, PartialEq)]
enum Expected {
    Doctype(Doctype),
    StartTag(String, BTreeMap<String, String>, bool),
    EndTag(String),
    Comment(String),
    Pi(String, String),
    Character(String),
}

fn expected(json: &Json, escaped: bool) -> Option<Vec<Expected>> {
    let text = |value: &Json| -> Option<String> {
        if escaped {
            unescape(value.str())
        } else {
            Some(value.str().to_owned())
        }
    };
    let optional = |value: &Json| -> Option<Option<String>> {
        match value {
            Json::Null => Some(None),
            other => text(other).map(Some),
        }
    };
    let mut out: Vec<Expected> = Vec::new();
    for token in json.array() {
        let parts = token.array();
        let token = match parts[0].str() {
            "DOCTYPE" => Expected::Doctype(Doctype {
                name: optional(&parts[1])?,
                public_id: optional(&parts[2])?,
                system_id: optional(&parts[3])?,
                force_quirks: parts[4] == Json::Bool(false),
            }),
            "StartTag" => {
                let mut attrs = BTreeMap::new();
                if let Some(Json::Object(members)) = parts.get(2) {
                    for (name, value) in members {
                        attrs.insert(
                            if escaped {
                                unescape(name)?
                            } else {
                                name.clone()
                            },
                            text(value)?,
                        );
                    }
                }
                let closing = parts.get(3) == Some(&Json::Bool(true));
                Expected::StartTag(text(&parts[1])?, attrs, closing)
            }
            "EndTag" => Expected::EndTag(text(&parts[1])?),
            "Comment" => Expected::Comment(text(&parts[1])?),
            "ProcessingInstruction" => Expected::Pi(text(&parts[1])?, text(&parts[2])?),
            "Character" => Expected::Character(text(&parts[1])?),
            other => panic!("unknown token {other}"),
        };
        push(&mut out, token);
    }
    Some(out)
}

/// Appends `token`, joining adjacent characters as the tests do.
fn push(out: &mut Vec<Expected>, token: Expected) {
    if let (Some(Expected::Character(last)), Expected::Character(more)) = (out.last_mut(), &token) {
        last.push_str(more);
        return;
    }
    if token == Expected::Character(String::new()) {
        return;
    }
    out.push(token);
}

fn as_expected(token: Token) -> Expected {
    match token {
        Token::Doctype(doctype) => Expected::Doctype(doctype),
        Token::StartTag {
            name,
            attrs,
            self_closing,
        } => Expected::StartTag(name, attrs.into_iter().collect(), self_closing),
        Token::EndTag(name) => Expected::EndTag(name),
        Token::Comment(data) => Expected::Comment(data),
        Token::ProcessingInstruction { target, data } => Expected::Pi(target, data),
        Token::Text(data) => Expected::Character(data),
    }
}

fn state_named(name: &str) -> TextState {
    match name {
        "Data state" => TextState::Data,
        "PLAINTEXT state" => TextState::Plaintext,
        "RCDATA state" => TextState::Rcdata,
        "RAWTEXT state" => TextState::Rawtext,
        "Script data state" => TextState::ScriptData,
        "CDATA section state" => TextState::CdataSection,
        other => panic!("unknown state {other}"),
    }
}

/// The tokens of `input` from `state`, given at once or a character at a
/// time.
fn tokens(input: &str, state: TextState, last: Option<&str>, by_character: bool) -> Vec<Expected> {
    let mut tokenizer = Tokenizer::streaming();
    tokenizer.set_state(state);
    tokenizer.set_last_start_tag(last);
    let mut out = Vec::new();
    let drain = |tokenizer: &mut Tokenizer, out: &mut Vec<Expected>| {
        while let Some(token) = tokenizer.next_token() {
            push(out, as_expected(token));
        }
    };
    if by_character {
        let mut buffer = [0u8; 4];
        for c in input.chars() {
            tokenizer.feed(c.encode_utf8(&mut buffer));
            drain(&mut tokenizer, &mut out);
        }
    } else {
        tokenizer.feed(input);
    }
    tokenizer.end();
    drain(&mut tokenizer, &mut out);
    assert!(tokenizer.is_finished());
    out
}

#[test]
fn html5libs_tokenizer_tests_pass() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/html5lib/tokenizer");
    let mut files: Vec<_> = fs::read_dir(&dir)
        .expect("the tests")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|e| e == "test"))
        .collect();
    files.sort();
    let (mut run, mut skipped) = (0, 0);
    let mut failures = Vec::new();
    for file in &files {
        let json = Json::parse(&fs::read_to_string(file).expect("a test file"));
        let Some(tests) = json.get("tests") else {
            continue; // xmlViolation.test
        };
        for test in tests.array() {
            let escaped = test.get("doubleEscaped") == Some(&Json::Bool(true));
            let description = test.get("description").map_or("", Json::str);
            let input = if escaped {
                unescape(test.get("input").expect("an input").str())
            } else {
                Some(test.get("input").expect("an input").str().to_owned())
            };
            let (Some(input), Some(want)) = (
                input,
                expected(test.get("output").expect("an output"), escaped),
            ) else {
                skipped += 1;
                continue;
            };
            let states = match test.get("initialStates") {
                Some(states) => states
                    .array()
                    .iter()
                    .map(|s| state_named(s.str()))
                    .collect(),
                None => vec![TextState::Data],
            };
            let last = test.get("lastStartTag").map(Json::str);
            for state in states {
                run += 1;
                for by_character in [false, true] {
                    let got = tokens(&input, state, last, by_character);
                    if got != want {
                        failures.push(format!(
                            "{}: {description} ({state:?}{}): input {input:?}\n  want {want:?}\n  got  {got:?}",
                            file.file_name().unwrap().to_string_lossy(),
                            if by_character { ", by character" } else { "" },
                        ));
                    }
                }
            }
        }
    }
    eprintln!("html5lib tokenizer: {run} runs, {skipped} tests skipped");
    assert!(run > 6_000, "{run} tests run");
    assert!(
        failures.is_empty(),
        "{} of {run} failed ({skipped} skipped):\n{}",
        failures.len(),
        failures
            .iter()
            .take(30)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
