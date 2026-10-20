use std::io::{Cursor, Read, Write};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use super::*;

wire_struct! {
    #[derive(Debug, Clone, PartialEq)]
    struct Sample {
        number: u32,
        name: String,
        list: Vec<u16>,
        maybe: Option<i64>,
        pair: (bool, f32),
        url: net::Url,
        body: Bytes,
        kind: Kind,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    One,
    Two,
}

wire_unit_enum!(Kind { 0 => Kind::One, 1 => Kind::Two });

wire_enum! {
    #[derive(Debug, Clone, PartialEq)]
    enum Shape {
        0 => Empty {},
        7 => Full { sample: Box<Sample>, more: Vec<Sample> },
    }
}

fn sample() -> Sample {
    Sample {
        number: 0xdead_beef,
        name: "na\u{ef}ve".into(),
        list: vec![1, 2, 65535],
        maybe: Some(-5),
        pair: (true, 1.5),
        url: net::Url::parse("https://a.test/p?q#f").expect("a URL"),
        body: Bytes(vec![0, 255, 10, 13]),
        kind: Kind::Two,
    }
}

#[test]
fn values_go_there_and_back() {
    let full = Shape::Full {
        sample: Box::new(sample()),
        more: vec![sample(), sample()],
    };
    for shape in [Shape::Empty {}, full] {
        let bytes = encode(&shape);
        assert_eq!(decode_all::<Shape>(&bytes), Ok(shape));
    }
    for value in [0u64, 1, u64::MAX] {
        assert_eq!(decode_all::<u64>(&encode(&value)), Ok(value));
    }
    for value in [i64::MIN, -1, 0, i64::MAX] {
        assert_eq!(decode_all::<i64>(&encode(&value)), Ok(value));
    }
    assert_eq!(decode_all::<usize>(&encode(&usize::MAX)), Ok(usize::MAX));
    let nan = decode_all::<f64>(&encode(&f64::NAN)).expect("a float");
    assert!(nan.is_nan(), "a float's bits are kept");
    assert_eq!(encode(&1u32), [1, 0, 0, 0], "little-endian");
    assert_eq!(encode(&"ab".to_owned()), [2, 0, 0, 0, b'a', b'b']);
}

#[test]
fn every_truncation_is_refused() {
    let bytes = encode(&Shape::Full {
        sample: Box::new(sample()),
        more: vec![sample()],
    });
    for end in 0..bytes.len() {
        assert!(
            decode_all::<Shape>(&bytes[..end]).is_err(),
            "{end} of {} bytes",
            bytes.len()
        );
    }
    let mut longer = bytes.clone();
    longer.push(0);
    assert_eq!(decode_all::<Shape>(&longer), Err(Malformed::Trailing));
}

#[test]
fn what_no_value_has_is_refused() {
    assert_eq!(
        decode_all::<Shape>(&[3, 0]),
        Err(Malformed::Tag("Shape", 3))
    );
    assert_eq!(decode_all::<bool>(&[2]), Err(Malformed::Invalid("bool")));
    assert_eq!(
        decode_all::<Option<u8>>(&[2, 0]),
        Err(Malformed::Tag("Option", 2))
    );
    assert_eq!(decode_all::<Kind>(&[9]), Err(Malformed::Tag("Kind", 9)));
    assert_eq!(
        decode_all::<String>(&[2, 0, 0, 0, 0xff, 0xfe]),
        Err(Malformed::Invalid("string"))
    );
    let mut url = Encoder::new();
    url.str("not a URL");
    assert_eq!(
        decode_all::<net::Url>(url.as_bytes()),
        Err(Malformed::Invalid("URL"))
    );
}

#[test]
fn a_count_is_held_to_the_bytes_left() {
    // Four billion values claimed, three bytes given: refused before any
    // room is made for them.
    let bytes = [0xff, 0xff, 0xff, 0xff, 1, 2, 3];
    assert_eq!(decode_all::<Vec<u8>>(&bytes), Err(Malformed::Truncated));
    assert_eq!(decode_all::<Vec<Sample>>(&bytes), Err(Malformed::Truncated));
    assert_eq!(decode_all::<Bytes>(&bytes), Err(Malformed::Truncated));
}

#[test]
fn messages_go_there_and_back() {
    let to_renderer = [
        ToRenderer::Hello {
            version: IPC_VERSION,
            process: 7,
        },
        ToRenderer::Shutdown {},
        ToRenderer::Fonts {
            primary: ("C:\\Windows\\Fonts\\segoeui.ttf".to_owned(), 0),
            fallbacks: vec![(0, ("msyh.ttc".to_owned(), 1))],
        },
    ];
    for message in to_renderer {
        assert_eq!(decode_all(&encode(&message)), Ok(message));
    }
    let hello = ToBrowser::Hello {
        version: IPC_VERSION,
    };
    assert_eq!(decode_all(&encode(&hello)), Ok(hello));
}

#[test]
fn frames_are_read_one_by_one() {
    let mut stream = Vec::new();
    for process in 0..3 {
        stream.extend(
            frame(&ToRenderer::Hello {
                version: 1,
                process,
            })
            .expect("short"),
        );
    }
    let mut reader = Cursor::new(stream);
    for process in 0..3 {
        let message = read_message::<ToRenderer>(&mut reader).expect("read");
        assert_eq!(
            message,
            Some(ToRenderer::Hello {
                version: 1,
                process
            })
        );
    }
    assert!(matches!(read_message::<ToRenderer>(&mut reader), Ok(None)));
}

#[test]
fn a_frame_past_the_limit_or_cut_short_stops_the_reading() {
    let too_long = ((MAX_MESSAGE + 1) as u32).to_le_bytes();
    assert!(matches!(
        read_message::<ToRenderer>(&mut Cursor::new(too_long)),
        Err(ReadError::TooLong(len)) if len == MAX_MESSAGE + 1
    ));
    // The longest frame allowed, claimed and not sent, allocates nothing
    // and ends as a cut frame.
    let claimed = (MAX_MESSAGE as u32).to_le_bytes();
    assert!(matches!(
        read_message::<ToRenderer>(&mut Cursor::new(claimed)),
        Err(ReadError::Io(_))
    ));
    let whole = frame(&ToRenderer::Shutdown {}).expect("short");
    for end in 1..whole.len() {
        assert!(
            matches!(
                read_message::<ToRenderer>(&mut Cursor::new(&whole[..end])),
                Err(ReadError::Io(_))
            ),
            "{end}"
        );
    }
    let mut bad = whole.clone();
    bad[4] = 0xee;
    assert!(matches!(
        read_message::<ToRenderer>(&mut Cursor::new(bad)),
        Err(ReadError::Malformed(Malformed::Tag("ToRenderer", 0xee)))
    ));
}

#[test]
fn a_message_too_long_is_never_sent() {
    let body = Bytes(vec![0; MAX_MESSAGE]);
    assert_eq!(frame(&body), None);
    let (writer, mut reader) = pipe();
    let sender = Sender::<Bytes>::new(writer);
    assert_eq!(sender.send(&body), Err(Closed));
    sender.close();
    let mut left = Vec::new();
    reader.read_to_end(&mut left).expect("read");
    assert!(left.is_empty(), "nothing of it written");
}

#[test]
fn a_pipe_ends_when_either_end_goes() {
    let (mut writer, mut reader) = pipe();
    writer.write_all(b"abc").expect("write");
    drop(writer);
    let mut read = Vec::new();
    reader.read_to_end(&mut read).expect("read");
    assert_eq!(read, b"abc");

    let (mut writer, reader) = pipe();
    drop(reader);
    assert_eq!(
        writer.write(b"x").map_err(|error| error.kind()),
        Err(std::io::ErrorKind::BrokenPipe)
    );
}

#[test]
fn a_reader_thread_delivers_and_says_why_it_stopped() {
    let ((browser_in, browser_out), (renderer_in, renderer_out)) = duplex();
    let to_renderer = Sender::<ToRenderer>::new(browser_out);
    let to_browser = Sender::<ToBrowser>::new(renderer_out);
    let (delivered, received) = mpsc::channel();
    let (stopped, why) = mpsc::channel();
    spawn_reader::<ToRenderer, _>(
        "test-renderer",
        renderer_in,
        move |message| delivered.send(message).expect("open"),
        move |reason| {
            stopped
                .send(reason.map(|reason| reason.to_string()))
                .expect("open")
        },
    )
    .expect("a thread");
    to_renderer
        .send(&ToRenderer::Hello {
            version: IPC_VERSION,
            process: 1,
        })
        .expect("sent");
    let timeout = Duration::from_secs(10);
    assert_eq!(
        received.recv_timeout(timeout),
        Ok(ToRenderer::Hello {
            version: IPC_VERSION,
            process: 1
        })
    );
    to_renderer.close();
    assert_eq!(why.recv_timeout(timeout), Ok(None), "a clean end");
    assert!(to_renderer.is_closed());
    drop(browser_in);
    assert_eq!(
        to_browser.send(&ToBrowser::Hello { version: 1 }),
        Err(Closed),
        "the reader is gone"
    );
    assert!(to_browser.is_closed(), "a failed write closes the channel");
}

#[test]
fn a_reader_thread_stops_at_a_malformed_message() {
    let (mut writer, reader) = pipe();
    let (stopped, why) = mpsc::channel();
    spawn_reader::<ToBrowser, _>(
        "test-browser",
        reader,
        |_| {},
        move |reason| {
            stopped
                .send(reason.map(|reason| reason.to_string()))
                .expect("open")
        },
    )
    .expect("a thread");
    writer.write_all(&[2, 0, 0, 0, 9, 9]).expect("write");
    assert_eq!(
        why.recv_timeout(Duration::from_secs(10)),
        Ok(Some(
            "a malformed message: unknown tag 2313 of ToBrowser".into()
        ))
    );
}

#[test]
fn calls_are_answered_by_their_numbers() {
    let ((browser_in, browser_out), (renderer_in, renderer_out)) = duplex();
    let calls = Arc::new(Calls::<u32>::new());
    // The browser answers each hello with its version plus one, through
    // the renderer's calls, as a reader thread delivers answers.
    let to_renderer = Sender::<ToRenderer>::new(browser_out);
    let answering = calls.clone();
    spawn_reader::<ToRenderer, _>(
        "test-answers",
        renderer_in,
        move |message| {
            if let ToRenderer::Hello { version, process } = message {
                assert!(answering.answer(u64::from(process), version));
            }
        },
        |_| {},
    )
    .expect("a thread");
    let to_browser = Sender::<ToBrowser>::new(renderer_out);
    let replies = to_renderer.clone();
    spawn_reader::<ToBrowser, _>(
        "test-asked",
        browser_in,
        move |message| {
            let ToBrowser::Hello { version } = message else {
                panic!("a hello: {message:?}");
            };
            // The question's number is in the version's high bits.
            let id = version >> 16;
            replies
                .send(&ToRenderer::Hello {
                    version: (version & 0xffff) + 1,
                    process: id,
                })
                .expect("open");
        },
        |_| {},
    )
    .expect("a thread");
    let (answered, answers) = mpsc::channel();
    for n in 0..8u32 {
        let calls = calls.clone();
        let to_browser = to_browser.clone();
        let answered = answered.clone();
        thread::spawn(move || {
            let answer = calls.call(&to_browser, |id| ToBrowser::Hello {
                version: ((id as u32) << 16) | n,
            });
            let _ = answered.send((n, answer));
        });
    }
    for _ in 0..8 {
        let (n, answer) = answers
            .recv_timeout(Duration::from_secs(10))
            .expect("answered in time");
        assert_eq!(answer, Ok(n + 1), "each its own answer");
    }
    assert_eq!(calls.waiting(), 0);
    assert!(!calls.answer(999, 0), "no question by that number");
}

#[test]
fn closing_the_calls_ends_those_that_wait() {
    let calls = Arc::new(Calls::<u32>::new());
    let (writer, _reader) = pipe();
    let sender = Sender::<ToBrowser>::new(writer);
    let waiting = {
        let calls = calls.clone();
        thread::spawn(move || calls.call(&sender, |_| ToBrowser::Hello { version: 1 }))
    };
    while calls.waiting() == 0 {
        thread::yield_now();
    }
    calls.close();
    assert_eq!(waiting.join().expect("joined"), Err(Closed));
    assert!(calls.begin().is_err(), "none asked again");
}

#[test]
fn a_console_entry_goes_there_and_back_whole() {
    use crate::{ConsoleEntry, ConsoleKind, ConsoleLevel, ConsoleSource, StackFrame};
    let entry = ConsoleEntry {
        level: ConsoleLevel::Warn,
        text: "t".to_owned(),
        source: ConsoleSource::Policy,
        kind: ConsoleKind::GroupCollapsed,
        time: 1234,
        url: "https://a.test/s.js".to_owned(),
        line: 7,
        column: 9,
        stack: vec![StackFrame {
            function: "f".to_owned(),
            url: "https://a.test/s.js".to_owned(),
            line: 7,
            column: 9,
        }],
        previews: "[]".to_owned(),
    };
    let message = ToBrowser::Console {
        view: 1,
        entries: vec![entry.clone(), ConsoleEntry::default()],
    };
    let ToBrowser::Console { entries, .. } =
        decode_all::<ToBrowser>(&encode(&message)).expect("a message")
    else {
        panic!("not the console's");
    };
    assert_eq!(entries, [entry, ConsoleEntry::default()]);
}
