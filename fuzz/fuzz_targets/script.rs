//! Any bytes as a classic script, under small limits, against a host
//! that records what it is asked. Nothing panics; a run ends, by
//! completing, throwing or being stopped by its step budget; the heap
//! stays under its limit; a dead realm runs nothing; and the host is
//! the engine's again after every run and checkpoint.

#![no_main]

use js::{ConsoleLevel, ErrorKind, Limits, QuickJs, Script, ScriptEngine, ScriptingHost};
use libfuzzer_sys::fuzz_target;

#[derive(Default)]
struct Recorder {
    calls: usize,
    title: String,
}

impl ScriptingHost for Recorder {
    fn console(&mut self, _: ConsoleLevel, _: String) {
        self.calls += 1;
    }

    fn title(&mut self) -> String {
        self.calls += 1;
        self.title.clone()
    }

    fn set_title(&mut self, title: String) {
        self.calls += 1;
        self.title = title;
    }
}

const LIMITS: Limits = Limits {
    heap: 4 << 20,
    steps: 50,
    stack: 64 << 10,
};

fuzz_target!(|data: &[u8]| {
    let source = String::from_utf8_lossy(data);
    let mut realm = QuickJs.realm::<Recorder>(LIMITS).expect("a realm");
    let mut host = Recorder::default();
    let script = Script {
        source: &source,
        url: "https://example.com/fuzz.html",
        line: 1,
        muted: false,
    };
    // Twice, so that a second run meets what the first left.
    for _ in 0..2 {
        let calls = host.calls;
        let result = realm.run(&script, &mut host);
        assert!(host.calls >= calls, "the host came back");
        let errors = realm.checkpoint(&mut host);
        assert!(realm.heap() <= LIMITS.heap, "the heap limit held");
        if realm.dead().is_some() {
            let again = realm.run(&script, &mut host).expect_err("a dead realm");
            assert_eq!(again.kind, ErrorKind::Dead);
            return;
        }
        if let Err(error) = result {
            assert_ne!(error.kind, ErrorKind::Dead);
        }
        assert!(errors.iter().all(|e| e.kind != ErrorKind::Dead));
    }
});
