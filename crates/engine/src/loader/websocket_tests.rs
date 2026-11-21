//! A page's WebSocket (G74), against a server of the test's own on
//! 127.0.0.1: the constructor's checks with the reference browser's
//! messages, the events of a connection in its order with their fields,
//! a connection refused, and a close before the connection opens.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use net::test_server::{Reply, TestServer};
use net::websocket::accept_for;

use super::tests::load_all;
use crate::testing::*;

/// Runs the engine's tasks as results come until `count` lines are
/// logged, or fails after ten seconds.
fn logged(tab: &mut Tab, count: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        tab.engine.poll_network();
        tab.engine.run_tasks();
        lines.extend(
            tab.engine
                .take_console()
                .into_iter()
                .map(|entry| entry.text),
        );
        if lines.len() >= count {
            return lines;
        }
        assert!(Instant::now() < deadline, "logged only {lines:?}");
        thread::sleep(Duration::from_millis(2));
    }
}

/// A page of `script`, scripting on, loaded.
fn page(script: &str) -> (Tab, TestServer) {
    let page = format!("<script>{script}</script>");
    let server = TestServer::plain(move |request| match request.target.as_str() {
        "/page.html" => Reply::ok("text/html", page.as_bytes()),
        _ => Reply::with_fields(404, &[("Content-Type", "text/plain")], b"no"),
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/page.html"));
    (tab, server)
}

/// A client's frame, unmasked: its first byte and payload.
fn read_frame(stream: &mut TcpStream) -> (u8, Vec<u8>) {
    let mut two = [0u8; 2];
    stream.read_exact(&mut two).expect("a frame");
    let len = usize::from(two[1] & 0x7f);
    assert!(len < 126, "a short frame");
    let mut mask = [0u8; 4];
    stream.read_exact(&mut mask).expect("its mask");
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).expect("its payload");
    for (i, b) in payload.iter_mut().enumerate() {
        *b ^= mask[i % 4];
    }
    (two[0], payload)
}

fn frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![0x80 | opcode, payload.len() as u8];
    out.extend_from_slice(payload);
    out
}

/// A WebSocket server for one connection: the handshake answered with
/// subprotocol `chat`, then `script`; its port.
fn serve(script: impl FnOnce(TcpStream) + Send + 'static) -> (u16, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("binds");
    let port = listener.local_addr().expect("an address").port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("a connection");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("a timeout");
        let mut head = Vec::new();
        let mut byte = [0u8];
        while !head.ends_with(b"\r\n\r\n") {
            assert_eq!(stream.read(&mut byte).expect("reads"), 1);
            head.push(byte[0]);
        }
        let head = String::from_utf8(head).expect("ASCII");
        let key = head
            .lines()
            .find_map(|line| line.strip_prefix("Sec-WebSocket-Key: "))
            .expect("a key");
        let answer = format!(
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\nSec-WebSocket-Protocol: chat\r\n\r\n",
            accept_for(key)
        );
        stream.write_all(answer.as_bytes()).expect("answers");
        script(stream);
    });
    (port, server)
}

#[test]
fn a_connection_fires_its_events_in_order_with_their_fields() {
    let (port, server) = serve(|mut stream| {
        assert_eq!(read_frame(&mut stream), (0x81, b"hello".to_vec()));
        assert_eq!(read_frame(&mut stream), (0x82, vec![1, 2, 3]));
        stream.write_all(&frame(0x1, b"hi")).unwrap();
        stream.write_all(&frame(0x2, &[7, 8])).unwrap();
        stream.write_all(&frame(0x2, &[9])).unwrap();
        let close = [&[0x03, 0xe8][..], b"bye"].concat();
        stream.write_all(&frame(0x8, &close)).unwrap();
        assert_eq!(read_frame(&mut stream), (0x88, close));
        let _ = stream.shutdown(Shutdown::Both);
    });
    let script = format!(
        "const w = new WebSocket('ws://127.0.0.1:{port}/chat', ['chat', 'other']);\
         console.log('made', w.readyState, w.url, w.binaryType, w.bufferedAmount);\
         let n = 0;\
         w.onopen = () => {{ console.log('open', w.readyState, w.protocol, JSON.stringify(w.extensions));\
           w.send('hello'); w.send(new Uint8Array([1, 2, 3])); console.log('buffered', w.bufferedAmount); }};\
         w.onmessage = (e) => {{ n++;\
           if (typeof e.data === 'string') {{ console.log('text', e.data, e.origin); w.binaryType = 'arraybuffer'; }}\
           else if (e.data instanceof ArrayBuffer) {{ console.log('buffer', new Uint8Array(e.data).join()); w.binaryType = 'blob'; }}\
           else {{ console.log('blob', e.data.size, e.data.constructor.name); }} }};\
         w.onerror = () => console.log('error');\
         w.onclose = (e) => console.log('close', e.code, e.reason, e.wasClean, w.readyState, e.constructor.name, n);"
    );
    let (mut tab, _pages) = page(&script);
    let lines = logged(&mut tab, 7);
    server.join().unwrap();
    assert_eq!(
        lines,
        [
            format!("made 0 ws://127.0.0.1:{port}/chat blob 0"),
            "open 1 chat \"\"".to_owned(),
            "buffered 8".to_owned(),
            format!("text hi ws://127.0.0.1:{port}"),
            "buffer 7,8".to_owned(),
            "blob 1 Blob".to_owned(),
            "close 1000 bye true 3 CloseEvent 3".to_owned(),
        ]
    );
}

#[test]
fn the_constructor_and_close_check_as_the_reference_says() {
    let script = "const t = (f) => { try { console.log('ok', f()); } catch (e) { console.log(e.name + ': ' + e.message); } };\
         t(() => new WebSocket('ftp://example.com/'));\
         t(() => new WebSocket('ws://127.0.0.1:1/#frag'));\
         t(() => new WebSocket('ws://127.0.0.1:1/', ['a', 'a']));\
         t(() => new WebSocket('ws://127.0.0.1:1/', ['a b']));\
         t(() => new WebSocket('http://127.0.0.1:1/p').url);\
         t(() => new WebSocket());\
         const w = new WebSocket('ws://127.0.0.1:1/');\
         t(() => w.send('x'));\
         t(() => w.close(1001));\
         t(() => w.close(undefined, 'r'));\
         t(() => w.close(66536));\
         t(() => w.close(1000, 'x'.repeat(124)));\
         t(() => { w.binaryType = 'text'; return w.binaryType; });\
         t(() => Object.getOwnPropertyNames(WebSocket.prototype).join());";
    let (mut tab, _pages) = page(script);
    let lines = logged(&mut tab, 13);
    let syntax = "SyntaxError: Failed to construct 'WebSocket': ";
    assert_eq!(
        lines[..13],
        [
            format!("{syntax}The URL's scheme must be either 'http', 'https', 'ws', or 'wss'. 'ftp' is not allowed."),
            format!("{syntax}The URL contains a fragment identifier ('frag'). Fragment identifiers are not allowed in WebSocket URLs."),
            format!("{syntax}The subprotocol 'a' is duplicated."),
            format!("{syntax}The subprotocol 'a b' is invalid."),
            "ok ws://127.0.0.1:1/p".to_owned(),
            "TypeError: Failed to construct 'WebSocket': 1 argument required, but only 0 present.".to_owned(),
            "InvalidStateError: Failed to execute 'send' on 'WebSocket': Still in CONNECTING state.".to_owned(),
            "InvalidAccessError: Failed to execute 'close' on 'WebSocket': The close code must be either 1000, or between 3000 and 4999. 1001 is neither.".to_owned(),
            "InvalidAccessError: Failed to execute 'close' on 'WebSocket': The close code must be either 1000, or between 3000 and 4999. 0 is neither.".to_owned(),
            "InvalidAccessError: Failed to execute 'close' on 'WebSocket': The close code must be either 1000, or between 3000 and 4999. 65535 is neither.".to_owned(),
            "SyntaxError: Failed to execute 'close' on 'WebSocket': The close reason must not be greater than 123 UTF-8 bytes.".to_owned(),
            "ok blob".to_owned(),
            "ok url,readyState,bufferedAmount,onopen,onerror,onclose,extensions,protocol,onmessage,binaryType,CONNECTING,OPEN,CLOSING,CLOSED,close,send,constructor".to_owned(),
        ]
    );
}

#[test]
fn a_refused_connection_and_a_close_before_it_opens_fail_as_the_reference_does() {
    // A port nothing listens on.
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").expect("binds");
        listener.local_addr().expect("an address").port()
    };
    let script = format!(
        "const v = new WebSocket('ws://127.0.0.1:{port}/');\
         v.onerror = (e) => console.log('error', e.constructor.name, v.readyState);\
         v.onclose = (e) => console.log('close', e.code, e.wasClean, JSON.stringify(e.reason), v.readyState);\
         const w = new WebSocket('ws://127.0.0.1:{port}/w');\
         w.onerror = () => console.log('w error');\
         w.onclose = (e) => console.log('w close', e.code, w.readyState);\
         w.close(); console.log('w closing', w.readyState);"
    );
    let (mut tab, _pages) = page(&script);
    let lines = logged(&mut tab, 7);
    let at = |line: &str| lines.iter().position(|l| l == line);
    // The page's own lines in the reference's order; the console's
    // errors among them.
    assert!(at("w closing 2") < at("w error"), "{lines:?}");
    assert!(at("w error") < at("w close 1006 3"), "{lines:?}");
    assert!(
        at("error Event 3") < at("close 1006 false \"\" 3"),
        "{lines:?}"
    );
    assert!(at("close 1006 false \"\" 3").is_some(), "{lines:?}");
    assert!(
        lines
            .iter()
            .any(|l| l == "WebSocket is closed before the connection is established."),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.starts_with(&format!(
            "WebSocket connection to 'ws://127.0.0.1:{port}/' failed: "
        ))),
        "{lines:?}"
    );
}

#[test]
fn a_dedicated_workers_websocket_works_as_a_windows_does() {
    let (port, server) = serve(|mut stream| {
        assert_eq!(read_frame(&mut stream), (0x81, b"from worker".to_vec()));
        stream.write_all(&frame(0x1, b"to worker")).unwrap();
        stream.write_all(&[0x88, 0x02, 0x0f, 0xa0]).unwrap();
        let _ = read_frame(&mut stream);
        let _ = stream.shutdown(Shutdown::Both);
    });
    let worker = format!(
        "const w = new WebSocket('ws://127.0.0.1:{port}/', 'chat');\
         w.onopen = () => {{ postMessage('open ' + w.protocol); w.send('from worker'); }};\
         w.onmessage = (e) => postMessage('message ' + e.data);\
         w.onclose = (e) => postMessage('close ' + e.code + ' ' + e.wasClean + ' ' + (e instanceof CloseEvent));"
    );
    let server_pages = TestServer::plain(move |request| match request.target.as_str() {
        "/page.html" => Reply::ok(
            "text/html",
            b"<script>new Worker('/w.js').onmessage = (e) => console.log(e.data);</script>",
        ),
        "/w.js" => Reply::ok("text/javascript", worker.as_bytes()),
        _ => Reply::with_fields(404, &[("Content-Type", "text/plain")], b"no"),
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server_pages.url("/page.html"));
    let lines = logged(&mut tab, 3);
    server.join().unwrap();
    assert_eq!(
        lines,
        ["open chat", "message to worker", "close 4000 true true"]
    );
}
