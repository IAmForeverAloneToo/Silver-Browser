//! The basic URL parser (URL Standard §4.4), as the state machine the
//! standard writes, with the state override the URL API's setters use
//! (gap batch A): a setter's run starts in its state on the URL as it
//! is, and its "return" and "return failure" both leave the URL as far
//! as the run changed it.
//!
//! The input is held as code points with a signed pointer, so that the
//! standard's "decrease pointer by 1" and "start over" read as written.

use encoding::{Encoding, UTF_8};

use super::host::{Host, parse_host};
use super::percent::{
    EncodeSet, percent_encode_after_encoding, utf8_percent_encode, utf8_percent_encode_char,
};
use super::{ParseError, Path, Record, default_port, is_special_scheme};

/// A state of the parser; a setter's run starts in one of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum State {
    SchemeStart,
    Scheme,
    NoScheme,
    SpecialRelativeOrAuthority,
    PathOrAuthority,
    Relative,
    RelativeSlash,
    SpecialAuthoritySlashes,
    SpecialAuthorityIgnoreSlashes,
    Authority,
    Host,
    Hostname,
    Port,
    File,
    FileSlash,
    FileHost,
    PathStart,
    Path,
    OpaquePath,
    Query,
    Fragment,
}

/// A Windows drive letter: an ASCII alpha and `:` or `|`.
fn is_windows_drive_letter(s: &[char]) -> bool {
    s.len() == 2 && s[0].is_ascii_alphabetic() && matches!(s[1], ':' | '|')
}

/// A normalized Windows drive letter: an ASCII alpha and `:`.
fn is_normalized_windows_drive_letter(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

/// Whether `s` starts with a Windows drive letter: one, alone or
/// followed by `/`, `\`, `?` or `#`.
fn starts_with_windows_drive_letter(s: &[char]) -> bool {
    s.len() >= 2
        && is_windows_drive_letter(&s[..2])
        && (s.len() == 2 || matches!(s[2], '/' | '\\' | '?' | '#'))
}

/// A single-dot URL path segment: `.` or `%2e`, in any case.
fn is_single_dot(segment: &str) -> bool {
    segment == "." || segment.eq_ignore_ascii_case("%2e")
}

/// A double-dot URL path segment: `..`, `.%2e`, `%2e.` or `%2e%2e`, in
/// any case.
fn is_double_dot(segment: &str) -> bool {
    matches!(
        segment.to_ascii_lowercase().as_str(),
        ".." | ".%2e" | "%2e." | "%2e%2e"
    )
}

impl Record {
    /// Shorten a URL's path: drop its last segment, unless it is a
    /// `file:` URL's lone drive letter.
    fn shorten_path(&mut self) {
        let is_file = self.scheme == "file";
        if let Path::Segments(segments) = &mut self.path {
            if is_file && segments.len() == 1 && is_normalized_windows_drive_letter(&segments[0]) {
                return;
            }
            segments.pop();
        }
    }

    fn segments(&mut self) -> &mut Vec<String> {
        match &mut self.path {
            Path::Segments(segments) => segments,
            Path::Opaque(_) => {
                unreachable!("only a URL without an opaque path reaches the path states")
            }
        }
    }

    fn special(&self) -> bool {
        is_special_scheme(&self.scheme)
    }

    fn includes_credentials(&self) -> bool {
        !self.username.is_empty() || !self.password.is_empty()
    }
}

/// How a run ended early: a failure, or a setter's run done.
enum Stop {
    Failure(ParseError),
    Done,
}

impl From<ParseError> for Stop {
    fn from(error: ParseError) -> Stop {
        Stop::Failure(error)
    }
}

/// Runs the basic URL parser over `input` with an optional `base` and
/// the document's `encoding` for the query.
pub(crate) fn parse(
    input: &str,
    base: Option<&Record>,
    encoding: Encoding,
) -> Result<Record, ParseError> {
    let mut url = Record::default();
    match run(input, base, encoding, &mut url, None) {
        Ok(()) | Err(Stop::Done) => Ok(url),
        Err(Stop::Failure(error)) => Err(error),
    }
}

/// Runs the basic URL parser over `input` with `url` and `state` as its
/// state override, as a setter of the URL API does: `url` as far as the
/// run changed it, whether it ended in failure or not.
pub(crate) fn parse_override(input: &str, url: &mut Record, state: State) {
    let _ = run(input, None, UTF_8, url, Some(state));
}

/// The basic URL parser's run, the state override `over` given or not.
fn run(
    input: &str,
    base: Option<&Record>,
    encoding: Encoding,
    url: &mut Record,
    over: Option<State>,
) -> Result<(), Stop> {
    let overridden = over.is_some();
    // Leading and trailing C0 controls and spaces go, unless a setter
    // runs it.
    let trimmed = match overridden {
        true => input,
        false => input.trim_matches(|c: char| c <= ' '),
    };
    let input: Vec<char> = trimmed
        .chars()
        .filter(|&c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    let mut encoding = encoding.output_encoding();
    let mut state = over.unwrap_or(State::SchemeStart);
    let mut buffer = String::new();
    let mut at_sign_seen = false;
    let mut inside_brackets = false;
    let mut password_token_seen = false;
    let len = input.len() as isize;
    let mut pointer: isize = 0;

    loop {
        // The code point the pointer points to, or `None` for EOF.
        let c = if (0..len).contains(&pointer) {
            Some(input[pointer as usize])
        } else {
            None
        };
        let remaining = |pointer: isize| -> &[char] {
            let start = (pointer + 1).clamp(0, len) as usize;
            &input[start..]
        };
        // The code points from the pointer on.
        let from_pointer = |pointer: isize| -> &[char] { &input[pointer.clamp(0, len) as usize..] };

        match state {
            State::SchemeStart => {
                if let Some(c) = c.filter(char::is_ascii_alphabetic) {
                    buffer.push(c.to_ascii_lowercase());
                    state = State::Scheme;
                } else if !overridden {
                    state = State::NoScheme;
                    pointer -= 1;
                } else {
                    return Err(Stop::Failure(ParseError::RelativeWithoutBase));
                }
            }
            State::Scheme => match c {
                Some(c) if c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.') => {
                    buffer.push(c.to_ascii_lowercase());
                }
                Some(':') => {
                    // A setter changes a scheme only to one of its kind,
                    // and none of a URL with credentials, a port or an
                    // empty host to `file`.
                    if overridden
                        && (url.special() != is_special_scheme(&buffer)
                            || ((url.includes_credentials() || url.port.is_some())
                                && buffer == "file")
                            || (url.scheme == "file" && url.host == Some(Host::Empty)))
                    {
                        return Err(Stop::Done);
                    }
                    url.scheme = std::mem::take(&mut buffer);
                    if overridden {
                        if url.port.is_some() && url.port == default_port(&url.scheme) {
                            url.port = None;
                        }
                        return Err(Stop::Done);
                    }
                    if url.scheme == "file" {
                        state = State::File;
                    } else if url.special() && base.is_some_and(|base| base.scheme == url.scheme) {
                        state = State::SpecialRelativeOrAuthority;
                    } else if url.special() {
                        state = State::SpecialAuthoritySlashes;
                    } else if remaining(pointer).first() == Some(&'/') {
                        state = State::PathOrAuthority;
                        pointer += 1;
                    } else {
                        url.path = Path::Opaque(String::new());
                        state = State::OpaquePath;
                    }
                }
                _ if !overridden => {
                    buffer.clear();
                    state = State::NoScheme;
                    // Start over, from the first code point.
                    pointer = -1;
                }
                _ => return Err(Stop::Failure(ParseError::RelativeWithoutBase)),
            },
            State::NoScheme => {
                let Some(base) = base else {
                    return Err(Stop::Failure(ParseError::RelativeWithoutBase));
                };
                let opaque = matches!(base.path, Path::Opaque(_));
                if opaque && c != Some('#') {
                    return Err(Stop::Failure(ParseError::RelativeWithoutBase));
                } else if opaque {
                    url.scheme = base.scheme.clone();
                    url.path = base.path.clone();
                    url.query = base.query.clone();
                    url.fragment = Some(String::new());
                    state = State::Fragment;
                } else if base.scheme != "file" {
                    state = State::Relative;
                    pointer -= 1;
                } else {
                    state = State::File;
                    pointer -= 1;
                }
            }
            State::SpecialRelativeOrAuthority => {
                if c == Some('/') && remaining(pointer).first() == Some(&'/') {
                    state = State::SpecialAuthorityIgnoreSlashes;
                    pointer += 1;
                } else {
                    state = State::Relative;
                    pointer -= 1;
                }
            }
            State::PathOrAuthority => {
                if c == Some('/') {
                    state = State::Authority;
                } else {
                    state = State::Path;
                    pointer -= 1;
                }
            }
            State::Relative => {
                let base = base.expect("the relative state has a base");
                url.scheme = base.scheme.clone();
                if c == Some('/') || (url.special() && c == Some('\\')) {
                    state = State::RelativeSlash;
                } else {
                    url.username = base.username.clone();
                    url.password = base.password.clone();
                    url.host = base.host.clone();
                    url.port = base.port;
                    url.path = base.path.clone();
                    url.query = base.query.clone();
                    match c {
                        Some('?') => {
                            url.query = Some(String::new());
                            state = State::Query;
                        }
                        Some('#') => {
                            url.fragment = Some(String::new());
                            state = State::Fragment;
                        }
                        Some(_) => {
                            url.query = None;
                            url.shorten_path();
                            state = State::Path;
                            pointer -= 1;
                        }
                        None => {}
                    }
                }
            }
            State::RelativeSlash => {
                if url.special() && matches!(c, Some('/' | '\\')) {
                    state = State::SpecialAuthorityIgnoreSlashes;
                } else if c == Some('/') {
                    state = State::Authority;
                } else {
                    let base = base.expect("the relative slash state has a base");
                    url.username = base.username.clone();
                    url.password = base.password.clone();
                    url.host = base.host.clone();
                    url.port = base.port;
                    state = State::Path;
                    pointer -= 1;
                }
            }
            State::SpecialAuthoritySlashes => {
                if c == Some('/') && remaining(pointer).first() == Some(&'/') {
                    pointer += 1;
                } else {
                    pointer -= 1;
                }
                state = State::SpecialAuthorityIgnoreSlashes;
            }
            State::SpecialAuthorityIgnoreSlashes => {
                if !matches!(c, Some('/' | '\\')) {
                    state = State::Authority;
                    pointer -= 1;
                }
            }
            State::Authority => {
                if c == Some('@') {
                    if at_sign_seen {
                        buffer.insert_str(0, "%40");
                    }
                    at_sign_seen = true;
                    for code_point in buffer.chars() {
                        if code_point == ':' && !password_token_seen {
                            password_token_seen = true;
                            continue;
                        }
                        let target = if password_token_seen {
                            &mut url.password
                        } else {
                            &mut url.username
                        };
                        utf8_percent_encode_char(code_point, EncodeSet::Userinfo, target);
                    }
                    buffer.clear();
                } else if matches!(c, None | Some('/' | '?' | '#'))
                    || (url.special() && c == Some('\\'))
                {
                    if at_sign_seen && buffer.is_empty() {
                        return Err(Stop::Failure(ParseError::EmptyHost));
                    }
                    pointer -= buffer.chars().count() as isize + 1;
                    buffer.clear();
                    state = State::Host;
                } else if let Some(c) = c {
                    buffer.push(c);
                }
            }
            State::Host | State::Hostname => {
                if overridden && url.scheme == "file" {
                    pointer -= 1;
                    state = State::FileHost;
                } else if c == Some(':') && !inside_brackets {
                    if buffer.is_empty() {
                        return Err(Stop::Failure(ParseError::EmptyHost));
                    }
                    // `hostname`'s setter takes no port.
                    if over == Some(State::Hostname) {
                        return Err(Stop::Done);
                    }
                    url.host = Some(parse_host(&buffer, !url.special())?);
                    buffer.clear();
                    state = State::Port;
                } else if matches!(c, None | Some('/' | '?' | '#'))
                    || (url.special() && c == Some('\\'))
                {
                    pointer -= 1;
                    if url.special() && buffer.is_empty() {
                        return Err(Stop::Failure(ParseError::EmptyHost));
                    }
                    // No empty host for a URL with credentials or a port.
                    if overridden
                        && buffer.is_empty()
                        && (url.includes_credentials() || url.port.is_some())
                    {
                        return Err(Stop::Done);
                    }
                    url.host = Some(parse_host(&buffer, !url.special())?);
                    buffer.clear();
                    state = State::PathStart;
                    if overridden {
                        return Err(Stop::Done);
                    }
                } else if let Some(c) = c {
                    if c == '[' {
                        inside_brackets = true;
                    }
                    if c == ']' {
                        inside_brackets = false;
                    }
                    buffer.push(c);
                }
            }
            State::Port => {
                if let Some(digit) = c.filter(char::is_ascii_digit) {
                    buffer.push(digit);
                } else if matches!(c, None | Some('/' | '?' | '#'))
                    || (url.special() && c == Some('\\'))
                    || overridden
                {
                    if !buffer.is_empty() {
                        // Leading zeros aside, more than five digits
                        // cannot be a 16-bit number.
                        let digits = buffer.trim_start_matches('0');
                        let port: u32 = match digits.len() {
                            0 => 0,
                            1..=5 => digits.parse().expect("one to five ASCII digits"),
                            _ => return Err(Stop::Failure(ParseError::InvalidPort)),
                        };
                        let port = u16::try_from(port).map_err(|_| ParseError::InvalidPort)?;
                        url.port = (default_port(&url.scheme) != Some(port)).then_some(port);
                        buffer.clear();
                        if overridden {
                            return Err(Stop::Done);
                        }
                    }
                    if overridden {
                        return Err(Stop::Failure(ParseError::InvalidPort));
                    }
                    state = State::PathStart;
                    pointer -= 1;
                } else {
                    return Err(Stop::Failure(ParseError::InvalidPort));
                }
            }
            State::File => {
                url.scheme = "file".to_owned();
                url.host = Some(Host::Empty);
                if matches!(c, Some('/' | '\\')) {
                    state = State::FileSlash;
                } else if let Some(base) = base.filter(|base| base.scheme == "file") {
                    url.host = base.host.clone();
                    url.path = base.path.clone();
                    url.query = base.query.clone();
                    match c {
                        Some('?') => {
                            url.query = Some(String::new());
                            state = State::Query;
                        }
                        Some('#') => {
                            url.fragment = Some(String::new());
                            state = State::Fragment;
                        }
                        Some(_) => {
                            url.query = None;
                            if !starts_with_windows_drive_letter(from_pointer(pointer)) {
                                url.shorten_path();
                            } else {
                                url.path = Path::Segments(Vec::new());
                            }
                            state = State::Path;
                            pointer -= 1;
                        }
                        None => {}
                    }
                } else {
                    state = State::Path;
                    pointer -= 1;
                }
            }
            State::FileSlash => {
                if matches!(c, Some('/' | '\\')) {
                    state = State::FileHost;
                } else {
                    if let Some(base) = base.filter(|base| base.scheme == "file") {
                        url.host = base.host.clone();
                        if !starts_with_windows_drive_letter(from_pointer(pointer))
                            && let Path::Segments(base_segments) = &base.path
                            && let Some(first) = base_segments.first()
                            && is_normalized_windows_drive_letter(first)
                        {
                            url.segments().push(first.clone());
                        }
                    }
                    state = State::Path;
                    pointer -= 1;
                }
            }
            State::FileHost => {
                if matches!(c, None | Some('/' | '\\' | '?' | '#')) {
                    pointer -= 1;
                    let buffer_chars: Vec<char> = buffer.chars().collect();
                    if !overridden && is_windows_drive_letter(&buffer_chars) {
                        // The buffer is kept: the path state takes it.
                        state = State::Path;
                    } else if buffer.is_empty() {
                        url.host = Some(Host::Empty);
                        if overridden {
                            return Err(Stop::Done);
                        }
                        state = State::PathStart;
                    } else {
                        let mut host = parse_host(&buffer, !url.special())?;
                        if host == Host::Domain("localhost".to_owned()) {
                            host = Host::Empty;
                        }
                        url.host = Some(host);
                        if overridden {
                            return Err(Stop::Done);
                        }
                        buffer.clear();
                        state = State::PathStart;
                    }
                } else if let Some(c) = c {
                    buffer.push(c);
                }
            }
            State::PathStart => {
                if url.special() {
                    state = State::Path;
                    if !matches!(c, Some('/' | '\\')) {
                        pointer -= 1;
                    }
                } else if !overridden && c == Some('?') {
                    url.query = Some(String::new());
                    state = State::Query;
                } else if !overridden && c == Some('#') {
                    url.fragment = Some(String::new());
                    state = State::Fragment;
                } else if let Some(c) = c {
                    state = State::Path;
                    if c != '/' {
                        pointer -= 1;
                    }
                } else if overridden && url.host.is_none() {
                    url.segments().push(String::new());
                }
            }
            State::Path => {
                let slash = c == Some('/') || (url.special() && c == Some('\\'));
                if c.is_none() || slash || (!overridden && matches!(c, Some('?' | '#'))) {
                    if is_double_dot(&buffer) {
                        url.shorten_path();
                        if !slash {
                            url.segments().push(String::new());
                        }
                    } else if is_single_dot(&buffer) && !slash {
                        url.segments().push(String::new());
                    } else if !is_single_dot(&buffer) {
                        let buffer_chars: Vec<char> = buffer.chars().collect();
                        if url.scheme == "file"
                            && url.segments().is_empty()
                            && is_windows_drive_letter(&buffer_chars)
                        {
                            buffer = format!("{}:", buffer_chars[0]);
                        }
                        let segment = std::mem::take(&mut buffer);
                        url.segments().push(segment);
                    }
                    buffer.clear();
                    if c == Some('?') {
                        url.query = Some(String::new());
                        state = State::Query;
                    }
                    if c == Some('#') {
                        url.fragment = Some(String::new());
                        state = State::Fragment;
                    }
                } else if let Some(c) = c {
                    utf8_percent_encode_char(c, EncodeSet::Path, &mut buffer);
                }
            }
            State::OpaquePath => {
                let Path::Opaque(path) = &mut url.path else {
                    unreachable!("the opaque path state has an opaque path");
                };
                match c {
                    Some('?') => {
                        url.query = Some(String::new());
                        state = State::Query;
                    }
                    Some('#') => {
                        url.fragment = Some(String::new());
                        state = State::Fragment;
                    }
                    Some(' ') => {
                        if matches!(remaining(pointer).first(), Some('?' | '#')) {
                            path.push_str("%20");
                        } else {
                            path.push(' ');
                        }
                    }
                    Some(c) => utf8_percent_encode_char(c, EncodeSet::C0Control, path),
                    None => {}
                }
            }
            State::Query => {
                if encoding != UTF_8
                    && (!url.special() || url.scheme == "ws" || url.scheme == "wss")
                {
                    encoding = UTF_8;
                }
                if c.is_none() || (!overridden && c == Some('#')) {
                    let set = if url.special() {
                        EncodeSet::SpecialQuery
                    } else {
                        EncodeSet::Query
                    };
                    let query = url.query.get_or_insert_with(String::new);
                    percent_encode_after_encoding(encoding, &buffer, set, query);
                    buffer.clear();
                    if c == Some('#') {
                        url.fragment = Some(String::new());
                        state = State::Fragment;
                    }
                } else if let Some(c) = c {
                    buffer.push(c);
                }
            }
            State::Fragment => {
                if let Some(c) = c {
                    let fragment = url.fragment.get_or_insert_with(String::new);
                    utf8_percent_encode_char(c, EncodeSet::Fragment, fragment);
                }
            }
        }

        if pointer >= len {
            break;
        }
        pointer += 1;
    }
    Ok(())
}

/// The fragment percent-encode set applied to a whole string, for a
/// fragment carried over from another URL's.
pub(crate) fn encode_fragment(fragment: &str) -> String {
    let mut out = String::with_capacity(fragment.len());
    utf8_percent_encode(fragment, EncodeSet::Fragment, &mut out);
    out
}
