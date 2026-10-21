//! HPACK against RFC 7541's examples (Appendix C) and its rules, and
//! the frames of RFC 9113 read, refused and written.

use super::frame::{self, ErrorCode, Frame, FrameError, MAX_FRAME_SIZE};
use super::hpack::{self, Decoder, Encoder, HpackError};

fn hex(text: &str) -> Vec<u8> {
    let digits: Vec<u8> = text.bytes().filter(u8::is_ascii_hexdigit).collect();
    digits
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

fn fields(list: &[(&str, &str)]) -> Vec<hpack::Field> {
    list.iter()
        .map(|(name, value)| (name.as_bytes().to_vec(), value.as_bytes().to_vec()))
        .collect()
}

fn decode(decoder: &mut Decoder, block: &[u8]) -> Result<Vec<hpack::Field>, HpackError> {
    decoder
        .decode(block, usize::MAX)
        .map(|decoded| decoded.fields)
}

#[test]
fn integers_and_literals_are_read_as_the_rfcs_examples_say() {
    // C.1: 10 and 1337 in a 5-bit prefix, after a literal's first byte.
    let mut decoder = Decoder::default();
    // C.2.1: a literal added to the table.
    let block = hex("400a 6375 7374 6f6d 2d6b 6579 0d63 7573 746f 6d2d 6865 6164 6572");
    assert_eq!(
        decode(&mut decoder, &block).unwrap(),
        fields(&[("custom-key", "custom-header")])
    );
    assert_eq!(decoder.table_size(), 55);
    // C.2.2: a literal not added.
    let mut decoder = Decoder::default();
    let block = hex("040c 2f73 616d 706c 652f 7061 7468");
    assert_eq!(
        decode(&mut decoder, &block).unwrap(),
        fields(&[(":path", "/sample/path")])
    );
    assert_eq!(decoder.table_size(), 0);
    // C.2.3: a literal never added.
    let block = hex("1008 7061 7373 776f 7264 0673 6563 7265 74");
    assert_eq!(
        decode(&mut decoder, &block).unwrap(),
        fields(&[("password", "secret")])
    );
    assert_eq!(decoder.table_size(), 0);
    // C.2.4: an indexed field.
    assert_eq!(
        decode(&mut decoder, &[0x82]).unwrap(),
        fields(&[(":method", "GET")])
    );
    // C.1.2: 1337, a multi-byte integer, as a literal's name index past
    // the tables is refused.
    assert!(decode(&mut decoder, &hex("1f9a0a")).is_err());
}

/// RFC 7541 C.3 (no Huffman) and C.4 (Huffman): three requests on one
/// connection, the dynamic table carried from each to the next.
const REQUESTS: [&[(&str, &str)]; 3] = [
    &[
        (":method", "GET"),
        (":scheme", "http"),
        (":path", "/"),
        (":authority", "www.example.com"),
    ],
    &[
        (":method", "GET"),
        (":scheme", "http"),
        (":path", "/"),
        (":authority", "www.example.com"),
        ("cache-control", "no-cache"),
    ],
    &[
        (":method", "GET"),
        (":scheme", "https"),
        (":path", "/index.html"),
        (":authority", "www.example.com"),
        ("custom-key", "custom-value"),
    ],
];

#[test]
fn the_rfcs_requests_are_decoded_in_turn() {
    let plain = [
        "8286 8441 0f77 7777 2e65 7861 6d70 6c65 2e63 6f6d",
        "8286 84be 5808 6e6f 2d63 6163 6865",
        "8287 85bf 400a 6375 7374 6f6d 2d6b 6579 0c63 7573 746f 6d2d 7661 6c75 65",
    ];
    let huffman = [
        "8286 8441 8cf1 e3c2 e5f2 3a6b a0ab 90f4 ff",
        "8286 84be 5886 a8eb 1064 9cbf",
        "8287 85bf 4088 25a8 49e9 5ba9 7d7f 8925 a849 e95b b8e8 b4bf",
    ];
    for blocks in [plain, huffman] {
        let mut decoder = Decoder::default();
        for ((block, request), size) in blocks.iter().zip(REQUESTS).zip([57, 110, 164]) {
            assert_eq!(decode(&mut decoder, &hex(block)).unwrap(), fields(request));
            assert_eq!(decoder.table_size(), size);
        }
    }
}

#[test]
fn the_encoder_writes_the_rfcs_huffman_requests() {
    let mut encoder = Encoder::default();
    let expected = [
        "8286 8441 8cf1 e3c2 e5f2 3a6b a0ab 90f4 ff",
        "8286 84be 5886 a8eb 1064 9cbf",
        "8287 85bf 4088 25a8 49e9 5ba9 7d7f 8925 a849 e95b b8e8 b4bf",
    ];
    for (request, block) in REQUESTS.iter().zip(expected) {
        let written = encoder.encode(
            request
                .iter()
                .map(|(name, value)| (name.as_bytes(), value.as_bytes())),
        );
        assert_eq!(written, hex(block));
    }
}

#[test]
fn huffman_strings_round_trip_and_bad_padding_is_refused() {
    // C.4.1's string.
    assert_eq!(
        hpack::huffman_encode(b"www.example.com"),
        hex("f1e3 c2e5 f23a 6ba0 ab90 f4ff")
    );
    let all: Vec<u8> = (0..=255).collect();
    assert_eq!(hpack::huffman_decode(&hpack::huffman_encode(&all)), Ok(all));
    assert_eq!(hpack::huffman_decode(&[]), Ok(Vec::new()));
    // Padding of zeros, or of a whole byte of ones past a symbol.
    let mut zeros = hpack::huffman_encode(b"a");
    *zeros.last_mut().unwrap() &= 0xf8;
    assert!(hpack::huffman_decode(&zeros).is_err());
    let mut long = hpack::huffman_encode(b"0");
    long.push(0xff);
    assert!(hpack::huffman_decode(&long).is_err());
    // The end of string, thirty ones, inside a string.
    assert!(hpack::huffman_decode(&[0xff, 0xff, 0xff, 0xff]).is_err());
}

#[test]
fn the_dynamic_table_stays_within_what_this_end_announced() {
    let mut decoder = Decoder::new(100);
    // A size update to 50, then a field of 55 bytes, which empties the
    // table rather than entering it.
    let mut block = vec![0x3f, 0x13];
    block.extend(hex(
        "400a 6375 7374 6f6d 2d6b 6579 0d63 7573 746f 6d2d 6865 6164 6572",
    ));
    assert_eq!(decode(&mut decoder, &block).unwrap().len(), 1);
    assert_eq!(decoder.table_size(), 0);
    // An update past the 100 announced, or after a field, is refused.
    assert!(decode(&mut decoder, &[0x3f, 0x60]).is_err());
    assert!(decode(&mut decoder, &[0x82, 0x20]).is_err());
    // Eviction keeps the newest.
    let mut decoder = Decoder::new(120);
    let mut encoder = Encoder::default();
    encoder.set_max_size(120);
    for value in ["aaaa", "bbbb", "cccc"] {
        let block = encoder.encode([(&b"x-name"[..], value.as_bytes())]);
        decode(&mut decoder, &block).unwrap();
        assert!(decoder.table_size() <= 120);
    }
    let block = encoder.encode([(&b"x-name"[..], &b"cccc"[..])]);
    assert_eq!(block.len(), 1, "indexed from the table");
    assert_eq!(
        decode(&mut decoder, &block).unwrap(),
        fields(&[("x-name", "cccc")])
    );
}

#[test]
fn a_list_past_its_bound_is_decoded_and_reported() {
    let mut encoder = Encoder::default();
    let mut decoder = Decoder::default();
    let long = "v".repeat(100);
    let list: Vec<(&[u8], &[u8])> = (0..10).map(|_| (&b"x-long"[..], long.as_bytes())).collect();
    let block = encoder.encode(list.clone());
    let decoded = decoder.decode(&block, 500).unwrap();
    assert!(decoded.too_large);
    assert_eq!(decoded.fields.len(), 3);
    // The table followed every field all the same: the next block's
    // index finds it.
    let again = encoder.encode(list);
    assert_eq!(decoder.decode(&again, usize::MAX).unwrap().fields.len(), 10);
}

#[test]
fn truncated_and_impossible_blocks_are_refused() {
    let mut decoder = Decoder::default();
    for block in [
        &[0x80][..],                                 // index 0
        &[0xff, 0xff, 0xff, 0xff, 0x7f],             // an index past both tables
        &[0x40, 0x05, b'a'],                         // a name cut short
        &[0x41],                                     // a value missing
        &[0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01], // too large an integer
        // Too long an integer, its bytes zeros (found by the fuzzer).
        &[
            0x7f, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x00,
        ],
    ] {
        assert!(decode(&mut decoder, block).is_err(), "{block:?}");
    }
}

#[test]
fn sensitive_fields_are_never_indexed() {
    let mut encoder = Encoder::default();
    let block = encoder.encode([(&b"authorization"[..], &b"secret"[..])]);
    assert_eq!(block[0] & 0xf0, 0x10, "never indexed");
    let again = encoder.encode([(&b"authorization"[..], &b"secret"[..])]);
    assert_eq!(again, block, "not in the table");
}

#[test]
fn frames_round_trip() {
    let frames = [
        Frame::Data {
            stream: 1,
            end_stream: true,
            data: b"hello".to_vec(),
            flow: 5,
        },
        Frame::Headers {
            stream: 3,
            end_stream: false,
            end_headers: true,
            block: vec![0x82, 0x86],
        },
        Frame::RstStream {
            stream: 5,
            code: ErrorCode::CANCEL,
        },
        Frame::Settings {
            ack: false,
            values: vec![
                (frame::setting::ENABLE_PUSH, 0),
                (frame::setting::INITIAL_WINDOW_SIZE, 1 << 20),
            ],
        },
        Frame::Settings {
            ack: true,
            values: Vec::new(),
        },
        Frame::Ping {
            ack: true,
            data: *b"12345678",
        },
        Frame::GoAway {
            last_stream: 7,
            code: ErrorCode::ENHANCE_YOUR_CALM,
            debug: b"calm".to_vec(),
        },
        Frame::WindowUpdate {
            stream: 0,
            increment: 65_535,
        },
        Frame::Continuation {
            stream: 3,
            end_headers: true,
            block: vec![0x84],
        },
        Frame::PushPromise {
            stream: 1,
            promised: 2,
            end_headers: true,
            block: vec![0x82],
        },
    ];
    for frame in frames {
        let bytes = frame.to_bytes();
        // Short of its last byte, it waits for more.
        assert_eq!(
            frame::read(&bytes[..bytes.len() - 1], MAX_FRAME_SIZE),
            Ok(None)
        );
        assert_eq!(
            frame::read(&bytes, MAX_FRAME_SIZE),
            Ok(Some((frame.clone(), bytes.len())))
        );
    }
    // An unknown type is read and set aside.
    let unknown = [0, 0, 1, 0xfa, 0, 0, 0, 0, 1, 7];
    assert_eq!(
        frame::read(&unknown, MAX_FRAME_SIZE),
        Ok(Some((
            Frame::Unknown {
                kind: 0xfa,
                stream: 1
            },
            10
        )))
    );
}

#[test]
fn padding_and_priority_are_taken_off() {
    // DATA padded: pad length 2, "hi", two bytes of padding.
    let bytes = [0, 0, 5, 0, 0x9, 0, 0, 0, 1, 2, b'h', b'i', 0, 0];
    let (frame, _) = frame::read(&bytes, MAX_FRAME_SIZE).unwrap().unwrap();
    assert_eq!(
        frame,
        Frame::Data {
            stream: 1,
            end_stream: true,
            data: b"hi".to_vec(),
            flow: 5,
        }
    );
    // HEADERS with a priority: its five bytes are not of the block.
    let bytes = [0, 0, 6, 1, 0x24, 0, 0, 0, 1, 0, 0, 0, 0, 16, 0x82];
    let (frame, _) = frame::read(&bytes, MAX_FRAME_SIZE).unwrap().unwrap();
    assert_eq!(
        frame,
        Frame::Headers {
            stream: 1,
            end_stream: false,
            end_headers: true,
            block: vec![0x82],
        }
    );
}

#[test]
fn frames_that_break_section_six_are_refused() {
    let connection = |bytes: &[u8]| {
        matches!(
            frame::read(bytes, MAX_FRAME_SIZE),
            Err(FrameError::Connection(..))
        )
    };
    // DATA on the connection.
    assert!(connection(&[0, 0, 1, 0, 0, 0, 0, 0, 0, 7]));
    // SETTINGS on a stream, of a broken length, or acknowledged with a
    // payload.
    assert!(connection(&[0, 0, 0, 4, 0, 0, 0, 0, 1]));
    assert!(connection(&[0, 0, 5, 4, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0]));
    assert!(connection(&[0, 0, 6, 4, 1, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0]));
    // PING of seven bytes.
    assert!(connection(&[
        0, 0, 7, 6, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7
    ]));
    // A window update of nothing on the connection; on a stream, that
    // stream's error.
    assert!(connection(&[0, 0, 4, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
    assert!(matches!(
        frame::read(&[0, 0, 4, 8, 0, 0, 0, 0, 3, 0, 0, 0, 0], MAX_FRAME_SIZE),
        Err(FrameError::Stream(3, ErrorCode::PROTOCOL_ERROR, _))
    ));
    // Padding longer than the frame.
    assert!(connection(&[0, 0, 2, 0, 0x8, 0, 0, 0, 1, 9, 0]));
    // A frame larger than allowed, known from its header alone.
    assert!(connection(&[0, 0x40, 1, 0, 0, 0, 0, 0, 1]));
}

#[test]
fn a_block_too_large_for_a_frame_goes_on_in_continuations() {
    let block: Vec<u8> = (0..40).collect();
    let mut out = Vec::new();
    frame::write_headers(&mut out, 1, &block, true, 16);
    let mut read = Vec::new();
    let mut at = 0;
    while let Some((frame, length)) = frame::read(&out[at..], MAX_FRAME_SIZE).unwrap() {
        read.push(frame);
        at += length;
    }
    assert_eq!(read.len(), 3);
    assert!(matches!(
        read[0],
        Frame::Headers {
            end_stream: true,
            end_headers: false,
            ..
        }
    ));
    assert!(matches!(
        read[1],
        Frame::Continuation {
            end_headers: false,
            ..
        }
    ));
    assert!(matches!(
        read[2],
        Frame::Continuation {
            end_headers: true,
            ..
        }
    ));
}

#[test]
fn a_smaller_table_is_announced_to_the_peer() {
    // The peer's decoder starts at the default; the encoder, told to keep
    // less, says so in its next block, and both evict alike.
    let mut encoder = Encoder::default();
    let mut decoder = Decoder::default();
    encoder.set_max_size(60);
    for value in ["first", "second", "third"] {
        let block = encoder.encode([(&b"x-name"[..], value.as_bytes())]);
        decode(&mut decoder, &block).unwrap();
        assert!(decoder.table_size() <= 60, "{}", decoder.table_size());
    }
    let block = encoder.encode([(&b"x-name"[..], &b"third"[..])]);
    assert_eq!(
        decode(&mut decoder, &block).unwrap(),
        fields(&[("x-name", "third")])
    );
}
