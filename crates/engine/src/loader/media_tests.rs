//! A media element's resource (M40 step 5): fetched by ranges into its
//! buffer and read as a file, a read waiting for its bytes; a seek far
//! ahead starts a range there; a server that answers no range sends the
//! whole; a response cut short is taken up again; a failure, or the
//! resource let go, ends the reads; the buffer keeps its bound.

use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;
use std::time::Duration;

use media::ByteSource;
use net::test_server::{Reply, ServerRequest, TestServer};
use net::{Client, Destination, Request};

use super::media::{MAX_MEDIA_BUFFER, MEDIA_RANGE, MediaError, MediaResource};

/// Bytes that are not all alike.
fn data(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 7 + i / 251) as u8).collect()
}

/// The bytes the request's `Range` asks for of `len`, as first and end.
fn asked_of(request: &ServerRequest, len: usize) -> (usize, usize) {
    let range = request
        .header("Range")
        .and_then(|range| range.strip_prefix("bytes="))
        .and_then(|range| range.split_once('-'));
    let first = range.and_then(|(first, _)| first.parse().ok()).unwrap_or(0);
    let last = range
        .and_then(|(_, last)| last.parse::<usize>().ok())
        .unwrap_or(len - 1)
        .min(len - 1);
    (first, last + 1)
}

/// The head of a 206 of the bytes from `first` to `end` of `len`.
fn head_206(len: usize, (first, end): (usize, usize)) -> Vec<u8> {
    format!(
        "HTTP/1.1 206 Partial\r\nContent-Type: video/webm\r\nContent-Range: bytes {first}-{}/{len}\r\nContent-Length: {}\r\n\r\n",
        end - 1,
        end - first
    )
    .into_bytes()
}

/// A 206 of what `request` asks of `data`, sent in pieces a little
/// apart.
fn slow_206(data: &[u8], request: &ServerRequest, pace: Duration) -> Reply {
    let (first, end) = asked_of(request, data.len());
    let mut pieces = vec![(Duration::ZERO, head_206(data.len(), (first, end)))];
    for piece in data[first..end].chunks(64 << 10) {
        pieces.push((pace, piece.to_vec()));
    }
    Reply::Pieces(pieces)
}

fn start(server: &TestServer, path: &str) -> MediaResource {
    MediaResource::start(
        Arc::new(Client::default()),
        Request::get(server.url(path), Destination::Video),
    )
}

/// The range asked for from `first`.
fn asked(first: u64) -> String {
    format!("bytes={first}-{}", first + MEDIA_RANGE - 1)
}

fn ranges_asked(server: &TestServer) -> Vec<String> {
    server
        .requests()
        .iter()
        .filter_map(|r| r.header("Range").map(str::to_owned))
        .collect()
}

#[test]
fn a_resource_is_read_whole_and_a_seek_far_ahead_starts_a_range_there() {
    let bytes = data(4 << 20);
    let served = bytes.clone();
    let server =
        TestServer::plain(move |request| slow_206(&served, request, Duration::from_millis(20)));
    let resource = start(&server, "/clip.webm");
    let mut reader = resource.reader();
    let mut head = [0u8; 100];
    reader.read_exact(&mut head).expect("read");
    assert_eq!(&head[..], &bytes[..100]);
    assert!(reader.seekable());
    assert_eq!(reader.length(), Some(4 << 20));
    // Far ahead of what has come: a range from there.
    let far = (3 << 20) + 12345;
    reader.seek(SeekFrom::Start(far)).expect("seek");
    let mut there = [0u8; 1000];
    reader.read_exact(&mut there).expect("read");
    assert_eq!(&there[..], &bytes[far as usize..far as usize + 1000]);
    assert_eq!(ranges_asked(&server), [asked(0), asked(far)]);
    // Then back to the start, still held, and on to the end.
    reader.seek(SeekFrom::Start(0)).expect("seek");
    let mut all = Vec::new();
    reader.read_to_end(&mut all).expect("read");
    assert_eq!(all, bytes);
    assert_eq!(resource.head().expect("a head").status, 206);
}

#[test]
fn a_server_that_answers_no_range_sends_the_whole() {
    let bytes = data(300_000);
    let served = bytes.clone();
    let server = TestServer::plain(move |_| Reply::ok("audio/ogg", &served));
    let resource = start(&server, "/a.ogg");
    let mut reader = resource.reader();
    reader.seek(SeekFrom::Start(299_000)).expect("seek");
    let mut tail = Vec::new();
    reader.read_to_end(&mut tail).expect("read");
    assert_eq!(tail, &bytes[299_000..]);
    assert_eq!(reader.length(), Some(300_000));
    assert!(reader.seekable(), "all of it held");
    reader.seek(SeekFrom::Start(0)).expect("seek");
    let mut all = Vec::new();
    reader.read_to_end(&mut all).expect("read");
    assert_eq!(all, bytes);
    assert_eq!(server.requests().len(), 1, "fetched once");
}

#[test]
fn a_ranged_response_cut_short_is_taken_up_again() {
    let bytes = data(200_000);
    let served = bytes.clone();
    let server = TestServer::plain(move |request| {
        let (first, end) = asked_of(request, served.len());
        // The first answer stops half way and closes.
        let cut = if first == 0 { 100_000 } else { end };
        let mut reply = head_206(served.len(), (first, end));
        reply.extend_from_slice(&served[first..cut]);
        Reply::SendAndClose(reply)
    });
    let resource = start(&server, "/clip.webm");
    let mut all = Vec::new();
    resource.reader().read_to_end(&mut all).expect("read");
    assert_eq!(all, bytes);
    assert_eq!(ranges_asked(&server), [asked(0), asked(100_000)]);
}

#[test]
fn a_failure_or_a_resource_let_go_ends_the_reads() {
    let server = TestServer::plain(|request| match request.target.as_str() {
        "/missing.webm" => Reply::with_fields(404, &[], b"no"),
        "/elsewhere.webm" => {
            Reply::with_fields(206, &[("Content-Range", "bytes 5-9/10")], b"later")
        }
        _ => Reply::Hang,
    });
    let missing = start(&server, "/missing.webm");
    // Read once the response has ended, its failure and its end both
    // known: the failure is what the read hears.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !missing.idle() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut buf = [0u8; 10];
    assert!(missing.reader().read(&mut buf).is_err());
    assert_eq!(
        missing.failed(),
        Some(MediaError::Network("status 404".into()))
    );
    // A range other than the one asked for.
    let elsewhere = start(&server, "/elsewhere.webm");
    assert!(elsewhere.reader().read(&mut buf).is_err());
    assert_eq!(
        elsewhere.failed(),
        Some(MediaError::Network("a range not the one asked for".into()))
    );
    let hanging = start(&server, "/hang.webm");
    let mut reader = hanging.reader();
    let reading = std::thread::spawn(move || reader.read(&mut [0u8; 10]).map_err(|e| e.kind()));
    std::thread::sleep(Duration::from_millis(200));
    drop(hanging);
    assert_eq!(
        reading.join().expect("joined"),
        Err(std::io::ErrorKind::Interrupted)
    );
}

#[test]
fn the_buffer_keeps_its_bound_letting_go_of_what_is_behind_the_reader() {
    let total = MAX_MEDIA_BUFFER + (16 << 20);
    let bytes = Arc::new(data(total));
    let served = bytes.clone();
    let server = TestServer::plain(move |request| {
        let (first, end) = asked_of(request, served.len());
        let mut reply = head_206(served.len(), (first, end));
        reply.extend_from_slice(&served[first..end]);
        Reply::Send(reply)
    });
    let resource = start(&server, "/big.webm");
    let mut reader = resource.reader();
    // Read a chunk at a time: the fetch waits for the reader past the
    // bound, and what is behind the reader goes.
    let mut chunk = vec![0u8; 1 << 20];
    let mut at = 0;
    let mut most = 0;
    while at < total {
        let n = reader.read(&mut chunk).expect("read");
        assert!(n > 0, "ended early at {at}");
        assert_eq!(&chunk[..n], &bytes[at..at + n]);
        at += n;
        most = most.max(resource.bytes_held());
    }
    assert!(most <= MAX_MEDIA_BUFFER + (2 << 20), "{most}");
    assert!(resource.held().first().is_some_and(|(start, _)| *start > 0));
}
