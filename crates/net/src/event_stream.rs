//! Server-sent events' stream (HTML section 9.2.5 and 9.2.6, G74): the
//! `text/event-stream` body parsed as it comes, a piece at a time, into
//! the events to dispatch and the reconnection times it sets. No I/O:
//! the engine's connection feeds it what its fetch delivers.
//!
//! The stream is UTF-8, decoded as it comes (a leading byte order mark
//! dropped, bytes that are not UTF-8 replaced, a character cut between
//! pieces kept for the next); its lines end with CRLF, LF or CR alone,
//! a CR at a piece's end waiting for the next to say whether an LF
//! follows it.

/// What a stream said, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Said {
    /// An event to dispatch: its type (`message` when none was named),
    /// its data, and the last event ID as it stood.
    Event {
        kind: String,
        data: String,
        last_event_id: String,
    },
    /// A reconnection time, in milliseconds.
    Retry(u64),
}

/// The most an event yet to be dispatched may hold, its data, type and
/// ID and the line being read together: past it, [`Parser::feed`]
/// fails. The stream as a whole has no bound, its events dispatched as
/// they end; one event is held whole first, and a server is not to make
/// a page hold more than this of one (the reference's bound is its
/// memory's).
pub const MAX_EVENT: usize = 64 * 1024 * 1024;

/// A stream's parser: its state between pieces.
#[derive(Debug, Default)]
pub struct Parser {
    /// Whether the stream's first bytes were seen, and its byte order
    /// mark (if any) dropped.
    started: bool,
    /// The bytes of a character cut at a piece's end.
    partial: Vec<u8>,
    /// The line so far.
    line: String,
    /// A CR ended the last piece: an LF first in the next is its pair.
    after_cr: bool,
    /// The data buffer: each `data` field's value and an LF.
    data: String,
    kind: String,
    /// The last event ID buffer, as fields set it.
    id_buffer: String,
    /// The last event ID string, as dispatches set it: what a
    /// reconnection sends.
    last_event_id: String,
}

/// The stream broke the bound: an event yet to be dispatched past
/// [`MAX_EVENT`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooLarge;

impl Parser {
    /// A parser whose last event ID is `last_event_id`: a reconnection's
    /// carries the one before.
    pub fn new(last_event_id: &str) -> Parser {
        Parser {
            id_buffer: last_event_id.to_owned(),
            last_event_id: last_event_id.to_owned(),
            ..Parser::default()
        }
    }

    /// The last event ID string as the last dispatch left it, for a
    /// reconnection's `Last-Event-ID`.
    pub fn last_event_id(&self) -> &str {
        &self.last_event_id
    }

    /// Parses `bytes`, the stream's next piece, appending what it said
    /// to `out`.
    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<Said>) -> Result<(), TooLarge> {
        if !self.started {
            // The byte order mark, perhaps cut between pieces.
            const BOM: &[u8] = b"\xef\xbb\xbf";
            self.partial.extend_from_slice(bytes);
            if self.partial.len() < BOM.len() && BOM.starts_with(&self.partial) {
                return Ok(());
            }
            self.started = true;
            let held = std::mem::take(&mut self.partial);
            let rest = held.strip_prefix(BOM).unwrap_or(&held);
            return self.decode(rest, out);
        }
        if self.partial.is_empty() {
            return self.decode(bytes, out);
        }
        // A character cut at the last piece's end, before this one.
        let mut held = std::mem::take(&mut self.partial);
        held.extend_from_slice(bytes);
        self.decode(&held, out)
    }

    /// Decodes `bytes` as UTF-8 and parses the text; an incomplete
    /// character at their end is kept.
    fn decode(&mut self, bytes: &[u8], out: &mut Vec<Said>) -> Result<(), TooLarge> {
        let mut rest = bytes;
        loop {
            match std::str::from_utf8(rest) {
                Ok(text) => return self.text(text, out),
                Err(e) => {
                    let (valid, after) = rest.split_at(e.valid_up_to());
                    // `valid_up_to` says this much is UTF-8.
                    let text = std::str::from_utf8(valid).unwrap_or_default();
                    self.text(text, out)?;
                    match e.error_len() {
                        Some(len) => {
                            self.text("\u{fffd}", out)?;
                            rest = &after[len..];
                        }
                        None => {
                            self.partial.extend_from_slice(after);
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    fn text(&mut self, text: &str, out: &mut Vec<Said>) -> Result<(), TooLarge> {
        let mut text = text;
        if self.after_cr && !text.is_empty() {
            self.after_cr = false;
            text = text.strip_prefix('\n').unwrap_or(text);
        }
        while let Some(at) = text.find(['\r', '\n']) {
            self.push(&text[..at])?;
            let line = std::mem::take(&mut self.line);
            self.line_done(&line, out)?;
            let cr = text.as_bytes()[at] == b'\r';
            text = &text[at + 1..];
            if cr {
                if text.is_empty() {
                    self.after_cr = true;
                } else {
                    text = text.strip_prefix('\n').unwrap_or(text);
                }
            }
        }
        self.push(text)
    }

    fn push(&mut self, text: &str) -> Result<(), TooLarge> {
        if self.pending() + text.len() > MAX_EVENT {
            return Err(TooLarge);
        }
        self.line.push_str(text);
        Ok(())
    }

    /// What the event yet to be dispatched holds, with the line being
    /// read.
    fn pending(&self) -> usize {
        self.line.len() + self.data.len() + self.kind.len() + self.id_buffer.len()
    }

    /// A whole line (section 9.2.6).
    fn line_done(&mut self, line: &str, out: &mut Vec<Said>) -> Result<(), TooLarge> {
        if line.is_empty() {
            self.dispatch(out);
            return Ok(());
        }
        if line.starts_with(':') {
            return Ok(());
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        let replacing = match field {
            "event" => self.kind.len(),
            "id" => self.id_buffer.len(),
            _ => 0,
        };
        if matches!(field, "event" | "id" | "data")
            && self.pending() - replacing + value.len() + 1 > MAX_EVENT
        {
            return Err(TooLarge);
        }
        match field {
            "event" => value.clone_into(&mut self.kind),
            "data" => {
                self.data.push_str(value);
                self.data.push('\n');
            }
            "id" => {
                if !value.contains('\0') {
                    value.clone_into(&mut self.id_buffer);
                }
            }
            // Past u64's range: as long as can be.
            "retry" if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) => {
                out.push(Said::Retry(value.parse().unwrap_or(u64::MAX)));
            }
            _ => {}
        }
        Ok(())
    }

    /// The blank line's dispatch (section 9.2.6, "dispatch the event").
    fn dispatch(&mut self, out: &mut Vec<Said>) {
        self.last_event_id.clone_from(&self.id_buffer);
        let kind = std::mem::take(&mut self.kind);
        if self.data.is_empty() {
            return;
        }
        let mut data = std::mem::take(&mut self.data);
        data.pop();
        out.push(Said::Event {
            kind: if kind.is_empty() {
                "message".to_owned()
            } else {
                kind
            },
            data,
            last_event_id: self.id_buffer.clone(),
        });
    }
}

#[cfg(test)]
#[path = "event_stream_tests.rs"]
mod tests;
