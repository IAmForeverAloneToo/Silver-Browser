//! A sampler of the scripts a realm runs, by their URLs (the lead's
//! ruling, 2026-11-18): every so many of the runtime's interrupt checks
//! it reads the URL of each script on the stack, which says whose
//! script burns a renderer's or a worker's time. Built with the
//! `js-sampler` feature alone, off by default and never shipped; set
//! on by `SILVER_JS_SAMPLE=<period>`, which a build without the feature
//! never reads.
//!
//! It is the js crate's third module that allows `unsafe`: QuickJS-ng's
//! interrupt handler is given no context, and the stack is read through
//! the context's raw pointer, by `JS_GetScriptOrModuleName`, which
//! walks the runtime's frames and reads each function's file name and
//! nothing else: no error is made, no getter, no `Error.stackTraceLimit`
//! nor `Error.prepareStackTrace` is read or called, so a page sees
//! nothing of it. Function names and lines would need QuickJS-ng's own
//! backtrace, which a page can observe, so they are not read here.

#![allow(unsafe_code)]

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ptr::NonNull;
use std::rc::Rc;
use std::time::{Duration, Instant};

use rquickjs::qjs;

/// The most stack levels read a sample.
const MAX_DEPTH: i32 = 64;

/// How often a sampler says, on the standard error, what it has found.
const REPORT_EVERY: Duration = Duration::from_secs(2);

thread_local! {
    /// The tests' period, in place of the environment's.
    static FORCED: Cell<Option<u32>> = const { Cell::new(None) };
}

/// A realm's sampler, of its runtime alone, on the thread that runs it.
pub(crate) struct Sampler {
    /// The realm's context while it lives: set when it is made and
    /// cleared as the realm drops ([`Sampler::detach`]), before its
    /// context and runtime go. The interrupt handler that ticks it runs
    /// only while a script of that runtime runs, on its thread, so the
    /// pointer it reads never outlives what it points to.
    context: Cell<Option<NonNull<qjs::JSContext>>>,
    period: u32,
    ticks: Cell<u32>,
    /// A sample under way: a tick within it takes none.
    busy: Cell<bool>,
    /// The samples by the URL of the topmost script, and by every URL on
    /// the stack, each once a sample.
    top: RefCell<HashMap<String, u64>>,
    within: RefCell<HashMap<String, u64>>,
    samples: Cell<u64>,
    reported: Cell<Option<Instant>>,
}

impl Sampler {
    /// A sampler taking one sample every `SILVER_JS_SAMPLE` interrupt
    /// checks, when that is set to a number from 1 (or the tests' period).
    pub(crate) fn from_env() -> Option<Rc<Sampler>> {
        let period = FORCED.with(Cell::get).or_else(|| {
            std::env::var("SILVER_JS_SAMPLE")
                .ok()?
                .trim()
                .parse::<u32>()
                .ok()
        })?;
        (period > 0).then(|| {
            Rc::new(Sampler {
                context: Cell::new(None),
                period,
                ticks: Cell::new(0),
                busy: Cell::new(false),
                top: RefCell::new(HashMap::new()),
                within: RefCell::new(HashMap::new()),
                samples: Cell::new(0),
                reported: Cell::new(None),
            })
        })
    }

    /// The realms made on this thread sample every `period` checks: for
    /// the tests, which set no environment.
    #[cfg(test)]
    pub(crate) fn force(period: Option<u32>) {
        FORCED.with(|forced| forced.set(period));
    }

    pub(crate) fn attach(&self, context: &rquickjs::Context) {
        self.context.set(Some(context.as_raw()));
    }

    pub(crate) fn detach(&self) {
        self.context.set(None);
    }

    /// One interrupt check: a sample every [`Sampler::period`], none
    /// while one is taken. What the handler returns is not its to say.
    pub(crate) fn tick(&self) {
        let n = self.ticks.get().wrapping_add(1);
        self.ticks.set(n);
        if !n.is_multiple_of(self.period) || self.busy.get() {
            return;
        }
        let Some(context) = self.context.get() else {
            return;
        };
        self.busy.set(true);
        let mut urls: Vec<String> = Vec::new();
        for level in 0..MAX_DEPTH {
            if let Some(url) = script_at(context, level) {
                urls.push(url);
            }
        }
        if let Some(first) = urls.first() {
            *self.top.borrow_mut().entry(first.clone()).or_insert(0) += 1;
        }
        urls.sort_unstable();
        urls.dedup();
        for url in urls {
            *self.within.borrow_mut().entry(url).or_insert(0) += 1;
        }
        self.samples.set(self.samples.get() + 1);
        self.report();
        self.busy.set(false);
    }

    /// The samples by the topmost script's URL, the most first.
    pub(crate) fn top(&self) -> Vec<(String, u64)> {
        let mut top: Vec<(String, u64)> = self
            .top
            .borrow()
            .iter()
            .map(|(url, n)| (url.clone(), *n))
            .collect();
        top.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        top
    }

    /// What it has found, every [`REPORT_EVERY`], on the standard error.
    fn report(&self) {
        let due = self
            .reported
            .get()
            .is_none_or(|at| at.elapsed() >= REPORT_EVERY);
        if !due {
            return;
        }
        self.reported.set(Some(Instant::now()));
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("(unnamed)");
        let top: Vec<String> = self
            .top()
            .into_iter()
            .take(6)
            .map(|(url, n)| format!("{n} {}", url.chars().take(120).collect::<String>()))
            .collect();
        eprintln!(
            "js-sampler [{name}] {} samples; top: {}",
            self.samples.get(),
            top.join(" | ")
        );
    }
}

/// The URL of the script `level` frames down the stack the context's
/// runtime runs, if that frame is a script's (not a native's).
fn script_at(context: NonNull<qjs::JSContext>, level: i32) -> Option<String> {
    // SAFETY: the context is the live realm's (Sampler::context's
    // argument), and this runs within an interrupt check of its runtime,
    // on its thread, where QuickJS-ng reads its frames itself.
    let atom = unsafe { qjs::JS_GetScriptOrModuleName(context.as_ptr(), level) };
    if atom == qjs::JS_ATOM_NULL {
        return None;
    }
    let mut length: qjs::size_t = 0;
    // SAFETY: the atom is the one just given, which this takes and frees
    // with the C string made of it; the string's bytes are copied before.
    let text = unsafe {
        let text = qjs::JS_AtomToCStringLen(context.as_ptr(), &mut length, atom);
        let copied = (!text.is_null()).then(|| {
            let bytes = std::slice::from_raw_parts(text.cast::<u8>(), length as usize);
            String::from_utf8_lossy(bytes).into_owned()
        });
        if !text.is_null() {
            qjs::JS_FreeCString(context.as_ptr(), text);
        }
        qjs::JS_FreeAtom(context.as_ptr(), atom);
        copied
    };
    text.filter(|url| !url.is_empty())
}
