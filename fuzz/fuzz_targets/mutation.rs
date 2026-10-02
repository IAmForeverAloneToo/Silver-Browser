//! The engine mutating its document (M13). The input is a page, its
//! HTML up to the first zero byte, and a script of mutations after it,
//! read by `engine::differential::Mutator`. The engine draws a frame
//! after every mutation, as it would between them in use; every eighth
//! and at the end, what it shows — every element's computed style,
//! every fragment, every pixel — must be what a fresh engine computes
//! for the document as it then stands.

#![no_main]

use std::cell::RefCell;
use std::sync::OnceLock;

use engine::Engine;
use engine::differential::{Mutator, compare_with_fresh};
use libfuzzer_sys::fuzz_target;

/// The most mutations one input applies: enough for any sequence the
/// invalidation could get wrong, few enough to keep the fuzzer quick.
const MAX_STEPS: usize = 64;
/// The frame: small, so the time goes into the engine.
const WIDTH: u32 = 64;
const HEIGHT: u32 = 48;

static FONTS: OnceLock<text::FontCollection> = OnceLock::new();

thread_local! {
    /// The engine under test and the fresh one it is compared with,
    /// made once: loading a document starts either over.
    static ENGINES: RefCell<(Engine, Engine)> =
        RefCell::new((Engine::with_fonts(fonts().clone()), Engine::with_fonts(fonts().clone())));
}

fn fonts() -> &'static text::FontCollection {
    FONTS.get_or_init(|| {
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses")
    })
}

fuzz_target!(|data: &[u8]| {
    let split = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    let Ok(html) = std::str::from_utf8(&data[..split]) else {
        return;
    };
    let mut script = data.get(split + 1..).unwrap_or(&[]);
    ENGINES.with(|engines| {
        let (engine, fresh) = &mut *engines.borrow_mut();
        engine.load_document(html);
        engine.frame(WIDTH, HEIGHT);
        let mut mutator = Mutator::new(engine.document());
        for step in 1..=MAX_STEPS {
            if !engine.mutate(|document| mutator.step(document, &mut script)) {
                break;
            }
            if step % 8 == 0 {
                if let Err(difference) = compare_with_fresh(engine, fresh, WIDTH, HEIGHT) {
                    panic!("after {step} mutations: {difference}");
                }
            } else {
                engine.frame(WIDTH, HEIGHT);
            }
        }
        if let Err(difference) = compare_with_fresh(engine, fresh, WIDTH, HEIGHT) {
            panic!("at the end: {difference}");
        }
    });
});
