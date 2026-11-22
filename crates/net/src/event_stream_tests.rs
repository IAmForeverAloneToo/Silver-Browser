//! The event stream's parser against what the reference browser
//! dispatched of the same streams (Chrome 157, probed 2026-11-22).

use super::*;

fn message(data: &str, id: &str) -> Said {
    Said::Event {
        kind: "message".to_owned(),
        data: data.to_owned(),
        last_event_id: id.to_owned(),
    }
}

/// What `pieces`, fed one by one, said.
fn parse(pieces: &[&[u8]]) -> Vec<Said> {
    let mut parser = Parser::new("");
    let mut out = Vec::new();
    for piece in pieces {
        parser.feed(piece, &mut out).expect("within bounds");
    }
    out
}

#[test]
fn fields_make_events_as_the_reference_dispatched_them() {
    let said = parse(&[
        b"retry: 300\nid: 1\ndata: a\ndata: b\n\n",
        b"event: custom\ndata: c\n\n",
        b": comment\n\n",
        b"id\ndata: d\n\n",
        b"data: tail-not-sent",
    ]);
    assert_eq!(
        said,
        vec![
            Said::Retry(300),
            message("a\nb", "1"),
            Said::Event {
                kind: "custom".to_owned(),
                data: "c".to_owned(),
                last_event_id: "1".to_owned(),
            },
            message("d", ""),
        ]
    );
}

#[test]
fn line_endings_a_bom_and_odd_fields_as_the_reference_has_them() {
    let said = parse(&[
        b"\xef\xbb\xbfdata: bom\r\rdata: cr\r\n\r\ndata:nospace\n\n",
        b"data: x\nretry: abc\nid: a\x00b\nevent\n\n",
        b"data\n\ndata:\ndata:\n\n",
        b"event: \ndata: emptytype\n\n",
        b"dat",
        b"a: split\n",
        b"\n",
        b"data: \xe2\x82",
        b"\xac euro\n\n",
        b"data: bad \xff utf8\n\n",
    ]);
    assert_eq!(
        said,
        vec![
            message("bom", ""),
            message("cr", ""),
            message("nospace", ""),
            message("x", ""),
            message("", ""),
            message("\n", ""),
            message("emptytype", ""),
            message("split", ""),
            message("\u{20ac} euro", ""),
            message("bad \u{fffd} utf8", ""),
        ]
    );
}

#[test]
fn a_cr_ending_one_piece_and_its_lf_starting_the_next_are_one_line_end() {
    assert_eq!(
        parse(&[b"data: a\r", b"\n\r", b"\ndata: b\n", b"\n"]),
        vec![message("a", ""), message("b", "")]
    );
    // Taken for two line ends, it would end the event between the two.
    assert_eq!(
        parse(&[b"data: a\r", b"\ndata: b\r\n\r\n"]),
        vec![message("a\nb", "")]
    );
    // Byte by byte, a BOM and a four-byte character among them.
    let stream = "\u{feff}data: \u{1f600}\r\n\r\nid: 7\ndata: z\n\n".as_bytes();
    let pieces: Vec<&[u8]> = stream.chunks(1).collect();
    assert_eq!(
        parse(&pieces),
        vec![message("\u{1f600}", ""), message("z", "7")]
    );
    // A BOM is dropped once, at the start alone: a later one starts a
    // field of another name, ignored.
    assert_eq!(
        parse(&[b"\xef\xbb", b"\xbfdata: a\n\n\xef\xbb\xbfdata: b\n\n"]),
        vec![message("a", "")]
    );
}

#[test]
fn a_reconnection_sends_the_id_the_last_dispatch_committed() {
    let mut parser = Parser::new("41");
    let mut out = Vec::new();
    assert_eq!(parser.last_event_id(), "41");
    parser.feed(b"id: 42\ndata: x\n\n", &mut out).unwrap();
    assert_eq!(parser.last_event_id(), "42");
    // An ID with no blank line after it is not committed.
    parser.feed(b"id: 43\n", &mut out).unwrap();
    assert_eq!(parser.last_event_id(), "42");
    // A blank line commits it, even with no data to dispatch.
    parser.feed(b"\n", &mut out).unwrap();
    assert_eq!(parser.last_event_id(), "43");
    assert_eq!(out, vec![message("x", "42")]);
}

#[test]
fn retry_takes_digits_alone() {
    assert_eq!(
        parse(&[b"retry: 10\nretry: 1x\nretry:\nretry: -1\nretry:99999999999999999999999\n"]),
        vec![Said::Retry(10), Said::Retry(u64::MAX)]
    );
}

#[test]
fn a_line_past_the_bound_is_refused() {
    let mut parser = Parser::new("");
    let mut out = Vec::new();
    let piece = vec![b'a'; MAX_EVENT / 2 + 1];
    assert_eq!(parser.feed(&piece, &mut out), Ok(()));
    assert_eq!(parser.feed(&piece, &mut out), Err(TooLarge));
}
