//! Reading an HTTP/1.1 response (RFC 9112): the status line, the header
//! fields, interim responses, and the body by the length rules of §6.3.

use std::fmt;
use std::io::{self, BufRead, BufReader, Read};

use super::{Limits, Response, Version};
use crate::headers::HeaderList;

/// Why a response could not be read.
#[derive(Debug)]
pub enum ReadError {
    /// The connection ended or failed before a single byte of a response
    /// arrived, so the server cannot have acted on the request: one sent
    /// on a reused connection may be sent again (RFC 9112 §9.3.1).
    NoResponse(Option<io::Error>),
    /// Reading failed after the response had begun.
    Io(io::Error),
    /// The connection ended inside the response: in its header, before a
    /// body's declared length, or before a chunked body's last chunk.
    Truncated,
    /// The response broke HTTP's syntax.
    Malformed(Malformed),
    /// The status lines and fields took more than the limit.
    HeadersTooLarge,
    /// The body is, or says it is, longer than the limit.
    BodyTooLarge,
    /// A 101: there is no upgrade a request asked for.
    SwitchingProtocols,
    /// The sink the body was handed to refused the rest of it (M24.8);
    /// why is the sink's to say.
    Stopped,
}

/// What part of a response broke HTTP's syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Malformed {
    /// Not `HTTP/1.1` or `HTTP/1.0`, a space and a three-digit status.
    StatusLine,
    /// A field line without a token name and a colon, or a value holding
    /// NUL or a CR that ends no line.
    FieldLine,
    /// `Content-Length` values that are not decimal numbers, or disagree.
    ContentLength,
    /// The chunked coding with parameters.
    TransferEncoding,
    /// A chunk's size line, or what follows its data, is not as RFC 9112
    /// §7.1 writes them.
    Chunk,
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadError::NoResponse(None) => f.write_str("the connection closed with no response"),
            ReadError::NoResponse(Some(e)) => write!(f, "no response: {e}"),
            ReadError::Io(e) => write!(f, "reading the response failed: {e}"),
            ReadError::Truncated => f.write_str("the connection closed inside the response"),
            ReadError::Malformed(what) => write!(f, "malformed response: {what:?}"),
            ReadError::HeadersTooLarge => f.write_str("the response's header is too large"),
            ReadError::BodyTooLarge => f.write_str("the response's body is too large"),
            ReadError::SwitchingProtocols => f.write_str("an unasked-for protocol switch"),
            ReadError::Stopped => f.write_str("the body's reader stopped"),
        }
    }
}

impl std::error::Error for ReadError {}

/// Where a response's body goes as it is read, when it is not read
/// whole into the response (M24.8).
pub trait BodySink {
    /// The final response's head has been read: whether its body is to
    /// be handed here, a piece at a time as it arrives.
    fn head(&mut self, status: u16, reason: &[u8], headers: &HeaderList) -> bool;
    /// A piece of the body, its transfer coding removed; an `Err`
    /// ([`ReadError::Stopped`]) stops the read.
    fn body(&mut self, bytes: &[u8]) -> Result<(), ReadError>;
}

/// Where a body read goes: into the response, or to a sink.
enum Out<'s> {
    Whole(Vec<u8>),
    Sink(&'s mut dyn BodySink),
}

impl Out<'_> {
    fn put(&mut self, bytes: &[u8]) -> Result<(), ReadError> {
        match self {
            Out::Whole(body) => {
                body.extend_from_slice(bytes);
                Ok(())
            }
            Out::Sink(sink) => sink.body(bytes),
        }
    }
}

/// Reads one response to a request from `reader`, interim (1xx)
/// responses skipped. `to_head` says the request was a `HEAD`, whose
/// response has no body whatever its fields say.
///
/// The body is read whole. Its length follows RFC 9112 §6.3:
///
/// 1. none for `HEAD`, 204 and 304;
/// 2. the chunked coding when it is the last transfer coding, its
///    extensions and trailer fields dropped;
/// 3. to the connection's end for any other transfer coding, or any in
///    an HTTP/1.0 response, whose framing §6.1 calls faulty;
/// 4. `Content-Length`, its values agreeing;
/// 5. to the connection's end.
///
/// A transfer coding overrides `Content-Length`. Such a message may be
/// an attempt at response splitting, so its connection is not reused.
///
/// A line may end in LF alone (§2.2). Obsolete line folding becomes a
/// space (§5.2). A field line beginning with white space that has no
/// field before it to continue is ignored, as §2.2 allows.
pub fn read_response<R: Read>(
    reader: &mut BufReader<R>,
    to_head: bool,
    limits: Limits,
) -> Result<Response, ReadError> {
    read_response_to(reader, to_head, limits, None)
}

/// [`read_response`], the final response's body handed to `sink` a
/// piece at a time as it is read if the sink asks for it at the head;
/// the response returned then has an empty body.
pub fn read_response_to<R: Read>(
    reader: &mut BufReader<R>,
    to_head: bool,
    limits: Limits,
    sink: Option<&mut dyn BodySink>,
) -> Result<Response, ReadError> {
    let mut input = Input {
        reader,
        received: false,
        header_budget: limits.header_bytes,
    };
    let head = loop {
        let head = read_head(&mut input)?;
        match head.status {
            101 => return Err(ReadError::SwitchingProtocols),
            100..=199 => continue,
            _ => break head,
        }
    };
    let mut out = Out::Whole(Vec::new());
    if let Some(sink) = sink
        && sink.head(head.status, &head.reason, &head.headers)
    {
        out = Out::Sink(sink);
    }
    let delimited = read_body(&mut input, &head, to_head, limits.body_bytes, &mut out)?;
    let body = match out {
        Out::Whole(body) => body,
        Out::Sink(_) => Vec::new(),
    };
    let close = head
        .headers
        .get_decode_split(b"Connection")
        .is_some_and(|options| options.iter().any(|o| o.eq_ignore_ascii_case("close")));
    Ok(Response {
        persistent: head.version == Version::Http11 && !close && delimited,
        version: head.version,
        status: head.status,
        reason: head.reason,
        headers: head.headers,
        body,
    })
}

/// A response's status line and fields.
struct Head {
    version: Version,
    status: u16,
    reason: Vec<u8>,
    headers: HeaderList,
}

/// The stream, whether a byte has come from it, and what is left of the
/// header limit.
struct Input<'a, R> {
    reader: &'a mut BufReader<R>,
    received: bool,
    header_budget: usize,
}

impl<R: Read> Input<'_, R> {
    /// The bytes buffered, read in if there are none; empty at the end.
    fn fill(&mut self) -> Result<&[u8], ReadError> {
        loop {
            match self.reader.fill_buf() {
                Ok(_) => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) if self.received => return Err(ReadError::Io(e)),
                Err(e) => return Err(ReadError::NoResponse(Some(e))),
            }
        }
        Ok(self.reader.buffer())
    }

    fn consume(&mut self, amount: usize) {
        self.reader.consume(amount);
        self.received |= amount > 0;
    }

    /// The end of the stream where something more was due.
    fn early_end(&self) -> ReadError {
        if self.received {
            ReadError::Truncated
        } else {
            ReadError::NoResponse(None)
        }
    }

    /// A line, without its LF or a CR before it, its bytes taken from
    /// `budget`; `None` at the end of the stream before any byte of it.
    fn read_line(
        &mut self,
        mut budget: Budget,
        too_large: fn() -> ReadError,
    ) -> Result<Option<Vec<u8>>, ReadError> {
        let mut line = Vec::new();
        loop {
            let left = match &budget {
                Budget::Header => self.header_budget,
                Budget::Other(left) => **left,
            };
            let available = self.fill()?;
            if available.is_empty() {
                return if line.is_empty() {
                    Ok(None)
                } else {
                    Err(self.early_end())
                };
            }
            let (take, ends) = match available.iter().position(|&b| b == b'\n') {
                Some(lf) => (lf + 1, true),
                None => (available.len(), false),
            };
            if take > left {
                return Err(too_large());
            }
            line.extend_from_slice(&available[..take]);
            self.consume(take);
            match &mut budget {
                Budget::Header => self.header_budget -= take,
                Budget::Other(left) => **left -= take,
            }
            if ends {
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                return Ok(Some(line));
            }
        }
    }

    /// Exactly `length` bytes onto `out`, as they come.
    fn copy_exact(&mut self, mut length: usize, out: &mut Out<'_>) -> Result<(), ReadError> {
        while length > 0 {
            let available = self.fill()?;
            if available.is_empty() {
                return Err(self.early_end());
            }
            let take = available.len().min(length);
            out.put(&available[..take])?;
            self.consume(take);
            length -= take;
        }
        Ok(())
    }

    /// Everything to the end of the stream onto `out`, as it comes, at
    /// most `limit` bytes.
    fn copy_to_end(&mut self, limit: usize, out: &mut Out<'_>) -> Result<(), ReadError> {
        let mut received = 0;
        loop {
            let available = self.fill()?;
            let take = available.len();
            if take == 0 {
                return Ok(());
            }
            if take > limit - received {
                return Err(ReadError::BodyTooLarge);
            }
            out.put(available)?;
            self.consume(take);
            received += take;
        }
    }
}

/// Which allowance a line's bytes are taken from.
enum Budget<'a> {
    /// The header limit, shared by every status line and field line.
    Header,
    /// Another count, such as a chunked body's framing.
    Other(&'a mut usize),
}

fn headers_too_large() -> ReadError {
    ReadError::HeadersTooLarge
}

fn body_too_large() -> ReadError {
    ReadError::BodyTooLarge
}

/// A status line and its fields, up to the empty line.
fn read_head<R: Read>(input: &mut Input<R>) -> Result<Head, ReadError> {
    // Empty lines before a status line are dropped (§9.2's allowance for
    // stray line endings).
    let status_line = loop {
        let line = input
            .read_line(Budget::Header, headers_too_large)?
            .ok_or_else(|| input.early_end())?;
        if !line.is_empty() {
            break line;
        }
    };
    let (version, status, reason) = parse_status_line(&status_line)?;
    let mut fields: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    loop {
        let line = input
            .read_line(Budget::Header, headers_too_large)?
            .ok_or(ReadError::Truncated)?;
        match line.first() {
            None => break,
            Some(b' ' | b'\t') => {
                // Obsolete line folding, joined by a space; or, with no
                // field to continue, a line to ignore.
                if let Some((_, value)) = fields.last_mut() {
                    let more = field_value(&line)?;
                    if !more.is_empty() {
                        if !value.is_empty() {
                            value.push(b' ');
                        }
                        value.extend_from_slice(more);
                    }
                }
            }
            Some(_) => {
                let colon = line
                    .iter()
                    .position(|&b| b == b':')
                    .ok_or(ReadError::Malformed(Malformed::FieldLine))?;
                let name = &line[..colon];
                if !super::is_token(name) {
                    return Err(ReadError::Malformed(Malformed::FieldLine));
                }
                let value = field_value(&line[colon + 1..])?;
                fields.push((name.to_vec(), value.to_vec()));
            }
        }
    }
    let mut headers = HeaderList::new();
    for (name, value) in &fields {
        headers.append(name, value);
    }
    Ok(Head {
        version,
        status,
        reason,
        headers,
    })
}

/// `HTTP/1.1` or `HTTP/1.0`, a space, three digits, and then nothing or
/// a space and a reason phrase.
fn parse_status_line(line: &[u8]) -> Result<(Version, u16, Vec<u8>), ReadError> {
    let malformed = ReadError::Malformed(Malformed::StatusLine);
    let (version, rest) = if let Some(rest) = line.strip_prefix(b"HTTP/1.1 ") {
        (Version::Http11, rest)
    } else if let Some(rest) = line.strip_prefix(b"HTTP/1.0 ") {
        (Version::Http10, rest)
    } else {
        return Err(malformed);
    };
    let (code, reason) = rest
        .split_at_checked(3)
        .ok_or(ReadError::Malformed(Malformed::StatusLine))?;
    if !code.iter().all(u8::is_ascii_digit) {
        return Err(malformed);
    }
    let reason = match reason {
        [] => &[][..],
        [b' ', reason @ ..] => reason,
        _ => return Err(malformed),
    };
    if reason.iter().any(|&b| b == b'\0' || b == b'\r') {
        return Err(malformed);
    }
    let status = code.iter().fold(0, |n, &d| n * 10 + u16::from(d - b'0'));
    Ok((version, status, reason.to_vec()))
}

/// A field line's value: without the white space around it, and holding
/// neither NUL nor a CR that ends no line.
fn field_value(raw: &[u8]) -> Result<&[u8], ReadError> {
    let is_ows = |b: &u8| *b == b' ' || *b == b'\t';
    let start = raw.iter().position(|b| !is_ows(b)).unwrap_or(raw.len());
    let end = raw
        .iter()
        .rposition(|b| !is_ows(b))
        .map_or(start, |last| last + 1);
    let value = &raw[start..end];
    if value.iter().any(|&b| b == b'\0' || b == b'\r') {
        return Err(ReadError::Malformed(Malformed::FieldLine));
    }
    Ok(value)
}

/// The body onto `out`, and whether its end was marked by the message
/// itself rather than by the connection's end.
fn read_body<R: Read>(
    input: &mut Input<R>,
    head: &Head,
    to_head: bool,
    limit: usize,
    out: &mut Out<'_>,
) -> Result<bool, ReadError> {
    if to_head || matches!(head.status, 204 | 304) {
        return Ok(true);
    }
    if let Some(codings) = head.headers.get_decode_split(b"Transfer-Encoding") {
        if head.version == Version::Http10 {
            input.copy_to_end(limit, out)?;
            return Ok(false);
        }
        let last = codings
            .last()
            .map_or("", |coding| coding.trim_matches([' ', '\t']));
        let (name, parameters) = match last.split_once(';') {
            Some((name, parameters)) => (name.trim_end_matches([' ', '\t']), Some(parameters)),
            None => (last, None),
        };
        if !name.eq_ignore_ascii_case("chunked") {
            input.copy_to_end(limit, out)?;
            return Ok(false);
        }
        if parameters.is_some() {
            return Err(ReadError::Malformed(Malformed::TransferEncoding));
        }
        read_chunked(input, limit, out)?;
        return Ok(!head.headers.contains(b"Content-Length"));
    }
    if let Some(value) = head.headers.get(b"Content-Length") {
        let length = content_length(&value)?;
        let length = usize::try_from(length)
            .ok()
            .filter(|&length| length <= limit)
            .ok_or(ReadError::BodyTooLarge)?;
        if let Out::Whole(body) = out {
            body.reserve(length.min(64 * 1024));
        }
        input.copy_exact(length, out)?;
        return Ok(true);
    }
    input.copy_to_end(limit, out)?;
    Ok(false)
}

/// The length the `Content-Length` values give: decimal digits, the same
/// number in each value of the comma-separated list. A number too large
/// to hold is `u64::MAX`, which no limit admits.
fn content_length(value: &[u8]) -> Result<u64, ReadError> {
    let mut length = None;
    for part in value.split(|&b| b == b',') {
        let part = field_value(part).map_err(|_| ReadError::Malformed(Malformed::ContentLength))?;
        if part.is_empty() || !part.iter().all(u8::is_ascii_digit) {
            return Err(ReadError::Malformed(Malformed::ContentLength));
        }
        let n = part.iter().fold(0u64, |n, &d| {
            n.saturating_mul(10).saturating_add(u64::from(d - b'0'))
        });
        if length.is_some_and(|length| length != n) {
            return Err(ReadError::Malformed(Malformed::ContentLength));
        }
        length = Some(n);
    }
    length.ok_or(ReadError::Malformed(Malformed::ContentLength))
}

/// A chunked body: chunks up to the last, then trailer fields, which are
/// dropped. The size lines and the line endings after each chunk's data
/// may take as many bytes as the body may; trailer fields count against
/// the header limit.
fn read_chunked<R: Read>(
    input: &mut Input<R>,
    limit: usize,
    out: &mut Out<'_>,
) -> Result<(), ReadError> {
    let mut received = 0;
    let mut framing = limit;
    loop {
        let line = input
            .read_line(Budget::Other(&mut framing), body_too_large)?
            .ok_or(ReadError::Truncated)?;
        let size = chunk_size(&line)?;
        if size == 0 {
            break;
        }
        let size = usize::try_from(size)
            .ok()
            .filter(|&size| size <= limit - received)
            .ok_or(ReadError::BodyTooLarge)?;
        input.copy_exact(size, out)?;
        received += size;
        let end = input
            .read_line(Budget::Other(&mut framing), body_too_large)?
            .ok_or(ReadError::Truncated)?;
        if !end.is_empty() {
            return Err(ReadError::Malformed(Malformed::Chunk));
        }
    }
    loop {
        let line = input
            .read_line(Budget::Header, headers_too_large)?
            .ok_or(ReadError::Truncated)?;
        if line.is_empty() {
            return Ok(());
        }
    }
}

/// A chunk's size: hex digits, then optional white space and extensions
/// after a `;`, which are ignored. A size too large to hold is
/// `u64::MAX`.
fn chunk_size(line: &[u8]) -> Result<u64, ReadError> {
    let digits = line.iter().take_while(|b| b.is_ascii_hexdigit()).count();
    if digits == 0 {
        return Err(ReadError::Malformed(Malformed::Chunk));
    }
    let rest = &line[digits..];
    let rest = &rest[rest
        .iter()
        .take_while(|&&b| b == b' ' || b == b'\t')
        .count()..];
    if !rest.is_empty() && rest[0] != b';' {
        return Err(ReadError::Malformed(Malformed::Chunk));
    }
    Ok(line[..digits].iter().fold(0u64, |n, &d| {
        let digit = u64::from(char::from(d).to_digit(16).expect("a hex digit"));
        n.saturating_mul(16).saturating_add(digit)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads `bytes` as the response to a GET (or a HEAD).
    fn read_with(bytes: &[u8], to_head: bool, limits: Limits) -> Result<Response, ReadError> {
        read_response(&mut BufReader::new(bytes), to_head, limits)
    }

    fn read(bytes: &[u8]) -> Result<Response, ReadError> {
        read_with(bytes, false, Limits::default())
    }

    fn body(bytes: &[u8]) -> Vec<u8> {
        read(bytes).expect("a response").body
    }

    fn malformed(bytes: &[u8]) -> Malformed {
        match read(bytes) {
            Err(ReadError::Malformed(what)) => what,
            other => panic!("{:?}: {other:?}", String::from_utf8_lossy(bytes)),
        }
    }

    fn is(result: Result<Response, ReadError>, expected: &str) -> bool {
        format!("{:?}", result.map(|_| ())).contains(expected)
    }

    #[test]
    fn a_simple_response() {
        let response =
            read(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 5\r\n\r\nhello")
                .unwrap();
        assert_eq!(response.version, Version::Http11);
        assert_eq!(response.status, 200);
        assert_eq!(response.reason, b"OK");
        assert_eq!(response.headers.get(b"content-type").unwrap(), b"text/html");
        assert_eq!(response.body, b"hello");
        assert!(response.persistent);
    }

    #[test]
    fn status_lines() {
        for good in [
            &b"HTTP/1.1 200 OK\r\n\r\n"[..],
            b"HTTP/1.1 200 \r\n\r\n",
            b"HTTP/1.1 200\r\n\r\n",
            b"HTTP/1.0 404 Not Found\n\n",
            b"\r\n\nHTTP/1.1 200 OK\r\n\r\n",
            b"HTTP/1.1 999 Odd but three digits \xFF\r\n\r\n",
        ] {
            assert!(read(good).is_ok(), "{:?}", String::from_utf8_lossy(good));
        }
        assert_eq!(read(b"HTTP/1.1 599\r\n\r\n").unwrap().status, 599);
        for bad in [
            &b"HTTP/2 200 OK\r\n\r\n"[..],
            b"HTTP/1.2 200 OK\r\n\r\n",
            b"HTTP/1.10 200 OK\r\n\r\n",
            b"http/1.1 200 OK\r\n\r\n",
            b"HTTP/1.1  200 OK\r\n\r\n",
            b"HTTP/1.1 20 OK\r\n\r\n",
            b"HTTP/1.1 2000 OK\r\n\r\n",
            b"HTTP/1.1 200OK\r\n\r\n",
            b"HTTP/1.1 +20 OK\r\n\r\n",
            b"HTTP/1.1 200 O\x00K\r\n\r\n",
            b"HTTP/1.1 200 O\rK\r\n\r\n",
            b"<html>\r\n\r\n",
        ] {
            assert_eq!(
                malformed(bad),
                Malformed::StatusLine,
                "{:?}",
                String::from_utf8_lossy(bad)
            );
        }
    }

    #[test]
    fn field_lines() {
        let response =
            read(b"HTTP/1.1 200 OK\r\nA:  spaced \t\r\nB:\r\nC:x\r\nA: 2\r\n\r\n").unwrap();
        let fields: Vec<(&[u8], &[u8])> = response.headers.iter().collect();
        assert_eq!(
            fields,
            [
                (&b"A"[..], &b"spaced"[..]),
                (b"B", b""),
                (b"C", b"x"),
                (b"A", b"2")
            ]
        );
        // Obsolete line folding becomes one space.
        let response =
            read(b"HTTP/1.1 200 OK\r\nA: one\r\n  two \r\n\tthree\r\nB: b\r\n\r\n").unwrap();
        assert_eq!(response.headers.get(b"a").unwrap(), b"one two three");
        // White space before the first field: the line is ignored.
        let response = read(b"HTTP/1.1 200 OK\r\n Evil: 1\r\nA: a\r\n\r\n").unwrap();
        assert!(!response.headers.contains(b"Evil"));
        assert_eq!(response.headers.len(), 1);
        for bad in [
            &b"HTTP/1.1 200 OK\r\nName : x\r\n\r\n"[..],
            b"HTTP/1.1 200 OK\r\nNo colon\r\n\r\n",
            b"HTTP/1.1 200 OK\r\n: empty name\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nA\xFF: x\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nA: x\x00y\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nA: x\ry\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nA: x\r\n y\x00\r\n\r\n",
        ] {
            assert_eq!(
                malformed(bad),
                Malformed::FieldLine,
                "{:?}",
                String::from_utf8_lossy(bad)
            );
        }
        // Other control characters and high bytes pass through.
        assert_eq!(
            read(b"HTTP/1.1 200 OK\r\nA: \x01\xE9\r\n\r\n")
                .unwrap()
                .headers
                .get(b"a")
                .unwrap(),
            b"\x01\xE9"
        );
    }

    #[test]
    fn interim_responses_are_skipped() {
        let response = read(
            b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 103 Early Hints\r\nLink: </s.css>\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\nx",
        )
        .unwrap();
        assert_eq!(response.status, 200);
        assert!(!response.headers.contains(b"Link"));
        assert_eq!(response.body, b"x");
        assert!(is(
            read(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: x\r\n\r\n"),
            "SwitchingProtocols"
        ));
    }

    #[test]
    fn responses_without_a_body() {
        let fields = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello";
        let response = read_with(fields, true, Limits::default()).unwrap();
        assert_eq!(response.body, b"");
        assert!(response.persistent);
        assert_eq!(
            body(b"HTTP/1.1 204 No Content\r\nContent-Length: 5\r\n\r\nhello"),
            b""
        );
        assert_eq!(body(b"HTTP/1.1 304 Not Modified\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n"), b"");
    }

    #[test]
    fn content_length() {
        assert_eq!(
            body(b"HTTP/1.1 200 OK\r\nContent-Length: 5, 5\r\n\r\nhelloEXTRA"),
            b"hello"
        );
        assert_eq!(
            body(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Length: 02\r\n\r\nhi"),
            b"hi"
        );
        assert_eq!(body(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"), b"");
        for bad in [
            &b"HTTP/1.1 200 OK\r\nContent-Length: 5, 6\r\n\r\nhello!"[..],
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Length: 6\r\n\r\nhello!",
            b"HTTP/1.1 200 OK\r\nContent-Length: abc\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nContent-Length:\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nContent-Length: +5\r\n\r\nhello",
            b"HTTP/1.1 200 OK\r\nContent-Length: 5,\r\n\r\nhello",
            b"HTTP/1.1 200 OK\r\nContent-Length: 0x5\r\n\r\nhello",
            b"HTTP/1.1 200 OK\r\nContent-Length: 5 5\r\n\r\nhello",
        ] {
            assert_eq!(
                malformed(bad),
                Malformed::ContentLength,
                "{:?}",
                String::from_utf8_lossy(bad)
            );
        }
        assert!(is(
            read(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nhello"),
            "Truncated"
        ));
        assert!(is(
            read(b"HTTP/1.1 200 OK\r\nContent-Length: 99999999999999999999999999\r\n\r\n"),
            "BodyTooLarge"
        ));
    }

    #[test]
    fn chunked() {
        let chunked = |body: &[u8]| {
            let mut bytes = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
            bytes.extend_from_slice(body);
            read(&bytes)
        };
        let response =
            chunked(b"5\r\nhello\r\n1;ext=\"a;b\" \r\n \r\nA\r\n0123456789\r\n0\r\n\r\n").unwrap();
        assert_eq!(response.body, b"hello 0123456789");
        assert!(response.persistent);
        // Upper-case and padded sizes, bare LF, extensions after white
        // space, and trailer fields, which are dropped.
        let response = chunked(b"0005 ; x\nhello\n00\nTrailer: t\n\n").unwrap();
        assert_eq!(response.body, b"hello");
        assert!(!response.headers.contains(b"Trailer"));
        assert_eq!(
            chunked(b"a\r\n0123456789\r\nF\r\n0123456789abcde\r\n0\r\n\r\n")
                .unwrap()
                .body
                .len(),
            25
        );
        for bad in [
            &b"x\r\n"[..],
            b"\r\n",
            b"5x\r\nhello\r\n0\r\n\r\n",
            b"5\r\nhelloXY\r\n0\r\n\r\n",
            b"-1\r\n",
        ] {
            assert!(
                is(chunked(bad), "Chunk"),
                "{:?}",
                String::from_utf8_lossy(bad)
            );
        }
        assert!(is(chunked(b"5\r\nhel"), "Truncated"));
        assert!(is(chunked(b"5\r\nhello\r\n"), "Truncated"));
        assert!(is(chunked(b"5\r\nhello\r\n0\r\n"), "Truncated"));
        assert!(is(chunked(b"FFFFFFFFFFFFFFFFFFFFFFFF\r\n"), "BodyTooLarge"));
    }

    #[test]
    fn transfer_codings() {
        let response = read(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip, Chunked\r\n\r\n2\r\nhi\r\n0\r\n\r\n",
        )
        .unwrap();
        assert_eq!(response.body, b"hi");
        assert!(response.persistent);
        // Not chunked last: the body runs to the connection's end.
        let response = read(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked, gzip\r\nContent-Length: 1\r\n\r\n2\r\nhi").unwrap();
        assert_eq!(response.body, b"2\r\nhi");
        assert!(!response.persistent);
        assert_eq!(
            malformed(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked;q=1\r\n\r\n0\r\n\r\n"),
            Malformed::TransferEncoding
        );
        // Both: the transfer coding wins, and the connection is not reused.
        let response = read(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nhi\r\n0\r\n\r\n").unwrap();
        assert_eq!(response.body, b"hi");
        assert!(!response.persistent);
        // An HTTP/1.0 response's transfer coding is faulty framing.
        let response =
            read(b"HTTP/1.0 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nhi\r\n0\r\n\r\n")
                .unwrap();
        assert_eq!(response.body, b"2\r\nhi\r\n0\r\n\r\n");
    }

    #[test]
    fn persistence() {
        let persistent = |bytes: &[u8]| read(bytes).unwrap().persistent;
        assert!(persistent(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"));
        assert!(persistent(
            b"HTTP/1.1 200 OK\r\nConnection: keep-alive\r\nContent-Length: 0\r\n\r\n"
        ));
        assert!(!persistent(
            b"HTTP/1.1 200 OK\r\nConnection: keep-alive, CLOSE\r\nContent-Length: 0\r\n\r\n"
        ));
        assert!(!persistent(
            b"HTTP/1.0 200 OK\r\nConnection: keep-alive\r\nContent-Length: 0\r\n\r\n"
        ));
        // Read to the connection's end.
        let response = read(b"HTTP/1.1 200 OK\r\n\r\nall of it").unwrap();
        assert_eq!(response.body, b"all of it");
        assert!(!response.persistent);
    }

    #[test]
    fn limits() {
        let head = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n";
        let limits = Limits {
            header_bytes: head.len(),
            body_bytes: 5,
        };
        let one_more = b"HTTP/1.1 200 OK \r\nContent-Length: 5\r\n\r\nhello";
        assert!(is(read_with(one_more, false, limits), "HeadersTooLarge"));
        let mut fits = head.to_vec();
        fits.extend_from_slice(b"hello");
        assert_eq!(read_with(&fits, false, limits).unwrap().body, b"hello");
        // Interim responses count.
        let mut interim = b"HTTP/1.1 100 \r\n\r\n".to_vec();
        interim.extend_from_slice(&fits);
        assert!(is(read_with(&interim, false, limits), "HeadersTooLarge"));
        let limits = Limits {
            header_bytes: 1024,
            body_bytes: 5,
        };
        for long in [
            &b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nhello!"[..],
            b"HTTP/1.1 200\r\n\r\nhello!",
            b"HTTP/1.1 200\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nhel\r\n3\r\nlo!\r\n0\r\n\r\n",
        ] {
            assert!(
                is(read_with(long, false, limits), "BodyTooLarge"),
                "{:?}",
                String::from_utf8_lossy(long)
            );
        }
        assert_eq!(
            read_with(b"HTTP/1.1 200\r\n\r\nhello", false, limits)
                .unwrap()
                .body,
            b"hello"
        );
    }

    /// A stream that fails with `error` once its bytes run out.
    struct Failing(&'static [u8], io::ErrorKind);

    impl Read for Failing {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.0.is_empty() {
                return Err(self.1.into());
            }
            let n = self.0.len().min(buf.len());
            buf[..n].copy_from_slice(&self.0[..n]);
            self.0 = &self.0[n..];
            Ok(n)
        }
    }

    #[test]
    fn nothing_received_is_told_apart() {
        assert!(is(read(b""), "NoResponse(None)"));
        let reset = read_response(
            &mut BufReader::new(Failing(b"", io::ErrorKind::ConnectionReset)),
            false,
            Limits::default(),
        );
        assert!(is(reset, "NoResponse(Some"));
        let reset = read_response(
            &mut BufReader::new(Failing(b"HTTP/1.1", io::ErrorKind::ConnectionReset)),
            false,
            Limits::default(),
        );
        assert!(is(reset, "Io("));
        assert!(is(read(b"HTTP/1.1 200 OK\r\n"), "Truncated"));
        assert!(is(read(b"HTTP/1.1 200"), "Truncated"));
        // Interrupted reads are tried again.
        struct Interrupting(bool);
        impl Read for Interrupting {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                self.0 = !self.0;
                if self.0 {
                    return Err(io::ErrorKind::Interrupted.into());
                }
                let reply = b"HTTP/1.1 204\r\n\r\n";
                buf[..reply.len()].copy_from_slice(reply);
                Ok(reply.len())
            }
        }
        let response = read_response(
            &mut BufReader::new(Interrupting(false)),
            false,
            Limits::default(),
        );
        assert_eq!(response.unwrap().status, 204);
    }

    /// A stream that hands out its bytes in pieces of the given sizes.
    struct Pieces<'a> {
        bytes: &'a [u8],
        sizes: std::iter::Cycle<std::vec::IntoIter<usize>>,
    }

    impl Read for Pieces<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = self
                .sizes
                .next()
                .unwrap_or(1)
                .max(1)
                .min(buf.len())
                .min(self.bytes.len());
            buf[..n].copy_from_slice(&self.bytes[..n]);
            self.bytes = &self.bytes[n..];
            Ok(n)
        }
    }

    /// A sink that takes the body when `wanted`, as it comes.
    struct Collect {
        wanted: bool,
        head: Option<(u16, Vec<u8>)>,
        body: Vec<u8>,
        pieces: usize,
        refuse_after: Option<usize>,
    }

    impl BodySink for Collect {
        fn head(&mut self, status: u16, reason: &[u8], _: &HeaderList) -> bool {
            self.head = Some((status, reason.to_vec()));
            self.wanted
        }

        fn body(&mut self, bytes: &[u8]) -> Result<(), ReadError> {
            if self.refuse_after.is_some_and(|n| self.pieces >= n) {
                return Err(ReadError::Stopped);
            }
            self.body.extend_from_slice(bytes);
            self.pieces += 1;
            Ok(())
        }
    }

    fn collect(wanted: bool) -> Collect {
        Collect {
            wanted,
            head: None,
            body: Vec::new(),
            pieces: 0,
            refuse_after: None,
        }
    }

    #[test]
    fn a_body_goes_to_a_sink_as_it_is_read() {
        let samples: [(&[u8], &[u8]); 3] = [
            (
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n",
                b"hello world",
            ),
            (b"HTTP/1.1 201 Made\r\nContent-Length: 11\r\n\r\nhello world", b"hello world"),
            (b"HTTP/1.0 200 OK\r\n\r\nto the end", b"to the end"),
        ];
        for (bytes, want) in samples {
            let pieces = Pieces {
                bytes,
                sizes: vec![3].into_iter().cycle(),
            };
            let mut sink = collect(true);
            let response = read_response_to(
                &mut BufReader::with_capacity(4, pieces),
                false,
                Limits::default(),
                Some(&mut sink),
            )
            .unwrap();
            assert!(response.body.is_empty(), "the sink had it");
            assert_eq!(sink.body, want);
            assert!(sink.pieces > 1, "in pieces: {}", sink.pieces);
        }
        // A sink that does not want it leaves it to the response; one told
        // the head of the final response alone.
        let bytes =
            b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 404 Gone\r\nContent-Length: 2\r\n\r\nno";
        let mut sink = collect(false);
        let response = read_response_to(
            &mut BufReader::new(&bytes[..]),
            false,
            Limits::default(),
            Some(&mut sink),
        )
        .unwrap();
        assert_eq!(response.body, b"no");
        assert_eq!(sink.head, Some((404, b"Gone".to_vec())));
        assert!(sink.body.is_empty());
        // A sink that refuses stops the read; the limit holds as it goes.
        let bytes = b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\nhello world";
        let mut sink = collect(true);
        sink.refuse_after = Some(1);
        let pieces = Pieces {
            bytes,
            sizes: vec![2].into_iter().cycle(),
        };
        let result = read_response_to(
            &mut BufReader::with_capacity(2, pieces),
            false,
            Limits::default(),
            Some(&mut sink),
        );
        assert!(is(result, "Stopped"));
        let limits = Limits {
            body_bytes: 5,
            ..Limits::default()
        };
        // The limit is on the whole of a chunked body, not on each chunk:
        // two of 60 bytes under a limit of 100, the framing well under it.
        let chunk = "x".repeat(60);
        let chunked = format!(
            "HTTP/1.1 200 OK
Transfer-Encoding: chunked

3c
{chunk}
3c
{chunk}
0

"
        );
        let hundred = Limits {
            body_bytes: 100,
            ..Limits::default()
        };
        assert!(is(
            read_with(chunked.as_bytes(), false, hundred),
            "BodyTooLarge"
        ));
        let result = read_response_to(
            &mut BufReader::new(chunked.as_bytes()),
            false,
            hundred,
            Some(&mut collect(true)),
        );
        assert!(is(result, "BodyTooLarge"));
        let bytes = b"HTTP/1.0 200 OK\r\n\r\nto the end";
        let result = read_response_to(
            &mut BufReader::new(&bytes[..]),
            false,
            limits,
            Some(&mut collect(true)),
        );
        assert!(is(result, "BodyTooLarge"));
    }

    #[test]
    fn pieces_read_as_the_whole_does() {
        let samples: [&[u8]; 4] = [
            b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nA: 1\r\n  folded\r\nTransfer-Encoding: chunked\r\n\r\n5;x\r\nhello\r\n0\r\nT: t\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\nhello world",
            b"HTTP/1.0 200 OK\r\n\r\nto the end",
            b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\nhello",
        ];
        for bytes in samples {
            let whole = format!("{:?}", read(bytes));
            for sizes in [vec![1], vec![2, 3], vec![7, 1, 13], vec![64]] {
                for capacity in [1, 3, 8192] {
                    let pieces = Pieces {
                        bytes,
                        sizes: sizes.clone().into_iter().cycle(),
                    };
                    let got = read_response(
                        &mut BufReader::with_capacity(capacity, pieces),
                        false,
                        Limits::default(),
                    );
                    assert_eq!(format!("{got:?}"), whole, "{sizes:?} {capacity}");
                }
            }
        }
    }
}
