//! Any bytes as a message's serialized form (M31): what one realm takes
//! from another, made again in a realm whose global listens for the
//! message and for the `messageerror` a form it cannot make is, every
//! value made read and cloned again. One realm takes the inputs while
//! it lives. Nothing panics; the run ends, by completing, throwing or
//! being stopped by its step budget; and the heap stays under its limit.

#![no_main]

use std::cell::RefCell;

use js::{
    ConsoleLevel, Limits, MessageTarget, QuickJs, Realm, Script, ScriptEngine, ScriptingHost, Task,
};
use libfuzzer_sys::fuzz_target;

#[derive(Default)]
struct Recorder {
    lines: usize,
}

impl ScriptingHost for Recorder {
    fn console(&mut self, _: ConsoleLevel, _: String) {
        self.lines += 1;
    }

    fn title(&mut self) -> String {
        String::new()
    }

    fn set_title(&mut self, _: String) {}
}

/// Small limits, so the time goes into the deserializer; room on the
/// heap for the bootstrap and for many inputs, the realm being kept.
const LIMITS: Limits = Limits {
    heap: 16 << 20,
    steps: 50,
    stack: js::MAX_SCRIPT_STACK,
};

/// The listeners: what came is read whole and cloned again.
const LISTEN: &str = "addEventListener('message', function (e) {\
  try { console.log(JSON.stringify(e.data)); } catch (x) {}\
  try { console.log(String(structuredClone(e.data))); } catch (x) {}\
  console.log(e.ports.length);\
});\
addEventListener('messageerror', function () { console.log('error'); });";

/// A realm with the listeners added.
fn listening() -> Box<dyn Realm<Recorder>> {
    let mut realm = QuickJs.realm::<Recorder>(LIMITS).expect("a realm");
    let script = Script {
        source: LISTEN,
        url: "https://example.com/clone.html",
        line: 1,
        muted: false,
    };
    realm
        .run(&script, &mut Recorder::default())
        .expect("the listeners are added");
    realm
}

thread_local! {
    /// One realm for the inputs, made again when one leaves it dead or
    /// its heap three quarters full: making it is most of an input's
    /// time.
    static REALM: RefCell<Option<Box<dyn Realm<Recorder>>>> = const { RefCell::new(None) };
}

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data).into_owned();
    REALM.with_borrow_mut(|kept| {
        let mut realm = match kept.take() {
            Some(realm) if realm.dead().is_none() && realm.heap() < LIMITS.heap / 4 * 3 => realm,
            _ => listening(),
        };
        deliver(&mut *realm, text);
        *kept = Some(realm);
    });
});

/// `text` delivered to `realm`'s global as a message, twice, so that the
/// second meets what the first left.
fn deliver(realm: &mut dyn Realm<Recorder>, text: String) {
    let mut host = Recorder::default();
    let task = Task::Message {
        target: MessageTarget::Global,
        data: text,
        origin: "https://example.com".to_owned(),
    };
    for _ in 0..2 {
        let _ = realm.enter(&task, &mut host);
        let _ = realm.checkpoint(&mut host);
        assert!(realm.heap() <= LIMITS.heap, "the heap limit held");
        if realm.dead().is_some() {
            return;
        }
    }
}
