//! A buffer's bytes copied for a native (G255 step 7a), two ways by the
//! input's first byte:
//!
//! - Raw: a buffer of any kind (plain, resizable then grown or shrunk,
//!   detached, shared, empty) and a range of it, any offset and length,
//!   copied by the copy itself in a context with SharedArrayBuffer. It
//!   is held to a model: a shared buffer refused, a detached one's none,
//!   a range within the buffer's length now its bytes exactly, any
//!   other refused, never cut short; nothing panics.
//! - Page: buffers and views the input makes, resizes and detaches, each
//!   given to `crypto.subtle.digest`; the bytes its native is given are
//!   the bytes the page read just before the call, and a resizable one
//!   is refused as the reference refuses it, before any native call.

#![no_main]
#![forbid(unsafe_code)]

use js::{
    Answer, Call, ConsoleLevel, CryptoCall, DomError, Limits, QuickJs, Script, ScriptEngine,
    ScriptingHost,
};
use libfuzzer_sys::fuzz_target;

/// The pattern a buffer is filled with: byte `i` is this of `i`.
fn pattern(i: usize) -> u8 {
    (i.wrapping_mul(7).wrapping_add(3) & 0xff) as u8
}

/// The raw mode's model of one copy.
fn raw(data: &[u8]) {
    let at = |i: usize| data.get(i).copied().unwrap_or(0);
    let size = usize::from(at(1) % 48);
    let to = usize::from(at(2) % 48);
    let source = match at(0) % 6 {
        0 => format!("const b = new ArrayBuffer({size}); fill(b); b"),
        1 => format!(
            "const b = new ArrayBuffer({size}, {{maxByteLength: 48}}); fill(b); b.resize({to}); b"
        ),
        2 => format!("const b = new ArrayBuffer({size}); fill(b); b.transfer(); b"),
        3 => format!("const b = new SharedArrayBuffer({size}); fill(b); b"),
        4 => "new ArrayBuffer(0)".to_owned(),
        _ => format!("const b = new ArrayBuffer({size}); fill(b); new Uint8Array(b)"),
    };
    let source = format!(
        "function fill(b) {{ const u = new Uint8Array(b); \
         for (let i = 0; i < u.length; i++) u[i] = (i * 7 + 3) & 255; }} {source}"
    );
    // Offsets and lengths: small, at the edges, past them, and not whole
    // numbers at all.
    let number = |byte: u8| -> f64 {
        match byte % 8 {
            0 => f64::from(byte >> 3),
            1 => 48.0,
            2 => f64::from(u32::MAX),
            3 => 9_007_199_254_740_991.0,
            4 => -1.0,
            5 => 0.5,
            6 => f64::NAN,
            _ => f64::from(byte >> 3) + f64::from(byte % 3),
        }
    };
    let (offset, length) = (number(at(3)), number(at(4)));
    let Some(copied) = js::copy_of_buffer(&source, offset, length) else {
        return;
    };
    let kind = at(0) % 6;
    // The buffer's length when copied, and its bytes.
    let now = match kind {
        0 | 3 => size,
        1 => to,
        _ => 0,
    };
    let whole = |n: f64| n.fract() == 0.0 && (0.0..=9_007_199_254_740_991.0).contains(&n);
    let want: Result<Vec<u8>, &str> = if kind == 3 || kind == 5 {
        Err("TypeError")
    } else if !whole(offset) || !whole(length) {
        Err("RangeError")
    } else {
        let (offset, length) = (offset as usize, length as usize);
        match offset.checked_add(length) {
            Some(end) if end <= now => {
                // A grown buffer's new bytes are zeros.
                let filled = if kind == 1 { size.min(to) } else { now };
                Ok((offset..end)
                    .map(|i| if i < filled { pattern(i) } else { 0 })
                    .collect())
            }
            _ => Err("RangeError"),
        }
    };
    match (copied, want) {
        (Ok(got), Ok(want)) => assert_eq!(got, want, "{source} at {offset} for {length}"),
        (Err(got), Err(want)) => assert!(
            got.starts_with(want),
            "{source} at {offset} for {length}: {got}, not {want}"
        ),
        (got, want) => panic!("{source} at {offset} for {length}: {got:?}, not {want:?}"),
    }
}

/// The page mode's host: what the page said it read, and what each
/// digest's native was given.
#[derive(Default)]
struct Recorder {
    expected: Option<String>,
    refusals: usize,
    checked: usize,
}

impl ScriptingHost for Recorder {
    fn console(&mut self, _: ConsoleLevel, text: String) {
        if let Some(hex) = text.strip_prefix("expect:") {
            assert!(
                self.expected.is_none(),
                "a digest that never reached its native"
            );
            self.expected = Some(hex.to_owned());
        } else if let Some(name) = text.strip_prefix("refused:") {
            assert_eq!(
                name, "TypeError resizable",
                "a resizable buffer refused otherwise"
            );
            self.refusals += 1;
        }
    }

    fn title(&mut self) -> String {
        String::new()
    }

    fn set_title(&mut self, _: String) {}

    fn dom(&mut self, call: Call) -> Result<Answer, DomError> {
        if let Call::Crypto(call) = call
            && let CryptoCall::Run { buffers, .. } = *call
        {
            let expected = self
                .expected
                .take()
                .expect("a digest the page did not announce");
            let given: String = buffers
                .first()
                .map(|b| b.iter().map(|x| format!("{x:02x}")).collect())
                .unwrap_or_default();
            assert_eq!(given, expected, "a digest's native given other bytes");
            self.checked += 1;
            return Ok(Answer::Nothing);
        }
        Err(DomError::not_supported())
    }
}

const LIMITS: Limits = Limits {
    heap: 32 << 20,
    steps: 2000,
    stack: js::MAX_SCRIPT_STACK,
    secure: true,
};

/// The page mode: a program of the input's ops.
fn page(data: &[u8]) {
    let mut program = String::from(
        "const s = crypto.subtle; const bufs = [new ArrayBuffer(0)]; const views = [];\n\
         const hex = (u) => { let t = ''; for (let i = 0; i < u.length; i++) \
           t += (u[i] < 16 ? '0' : '') + u[i].toString(16); return t; };\n\
         function fill(b) { const u = new Uint8Array(b); \
           for (let i = 0; i < u.length; i++) u[i] = (i * 7 + 3) & 255; }\n\
         function check(x) {\n\
           const plain = x instanceof ArrayBuffer; const buf = plain ? x : x.buffer;\n\
           if (buf.resizable) { s.digest('SHA-256', x).catch((e) => \
             console.log('refused:' + e.name + (/must not be resizable/.test(e.message) ? ' resizable' : ''))); return; }\n\
           const u = buf.detached ? new Uint8Array(0) : plain ? new Uint8Array(x) \
             : new Uint8Array(buf, x.byteOffset, x.byteLength);\n\
           console.log('expect:' + hex(u)); s.digest('SHA-256', x);\n\
         }\n",
    );
    for op in data.chunks(3).take(24) {
        let [kind, a, b] = [op[0], *op.get(1).unwrap_or(&0), *op.get(2).unwrap_or(&0)];
        let line = match kind % 8 {
            0 => format!(
                "{{ const b = new ArrayBuffer({}); fill(b); bufs.push(b); }}",
                a % 40
            ),
            1 => format!(
                "{{ const b = new ArrayBuffer({}, {{maxByteLength: 40}}); fill(b); bufs.push(b); }}",
                a % 40
            ),
            2 => format!(
                "try {{ const b = bufs[{} % bufs.length]; if (b.resizable) b.resize({}); }} catch {{}}",
                a,
                b % 41
            ),
            3 => format!("try {{ bufs[{a} % bufs.length].transfer(); }} catch {{}}"),
            4 => format!(
                "try {{ views.push(new Uint8Array(bufs[{a} % bufs.length], {}, {})); }} catch {{}}",
                b % 41,
                b / 41 % 8
            ),
            5 => format!(
                "try {{ views.push(new Uint8Array(bufs[{a} % bufs.length], {})); }} catch {{}}",
                b % 41
            ),
            6 => format!(
                "try {{ views.push(new DataView(bufs[{a} % bufs.length], {})); }} catch {{}}",
                b % 41
            ),
            _ => format!(
                "check({} ? bufs[{a} % bufs.length] : views.length ? views[{a} % views.length] : bufs[0]);",
                b % 2
            ),
        };
        program.push_str(&line);
        program.push('\n');
    }
    let mut realm = QuickJs.realm::<Recorder>(LIMITS).expect("a realm");
    let mut host = Recorder::default();
    let script = Script {
        source: &program,
        url: "https://example.com/fuzz.html",
        line: 1,
        muted: false,
        base: "",
        options: 0,
    };
    // A run stopped by its budget leaves a digest unannounced to none.
    if realm.run(&script, &mut host).is_ok() {
        assert!(
            host.expected.is_none(),
            "a digest that never reached its native"
        );
        let _ = realm.checkpoint(&mut host);
    }
}

fuzz_target!(|data: &[u8]| {
    // On a thread of the engine's stack, which a realm needs for its
    // scripts (G253): libFuzzer's thread has the system's.
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(engine::STACK_SIZE)
            .spawn_scoped(scope, || match data.split_first() {
                Some((mode, rest)) if mode % 2 == 0 => raw(rest),
                Some((_, rest)) => page(rest),
                None => {}
            })
            .expect("a thread")
            .join()
            .expect("the run");
    });
});
