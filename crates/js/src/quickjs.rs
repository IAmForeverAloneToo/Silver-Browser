//! The QuickJS-ng backend (roadmap D1), through `rquickjs`'s safe
//! interface: a runtime and a context a realm, the host moved into a
//! slot the native functions read for the length of a run, and the
//! limits set on the runtime.

pub(crate) mod console;
mod dom;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rquickjs::context::EvalOptions;
use rquickjs::function::Rest;
use rquickjs::{
    CatchResultExt, CaughtError, Coerced, Context, Ctx, Exception, Function, Object, Persistent,
    Runtime, Value,
};

use crate::{
    Completion, ConsoleLevel, Death, ErrorKind, Limits, Realm, Script, ScriptEngine, ScriptError,
    ScriptingHost, Task,
};

/// QuickJS-ng 0.16, which `rquickjs` builds from its source.
#[derive(Debug, Clone, Copy, Default)]
pub struct QuickJs;

impl ScriptEngine for QuickJs {
    fn realm<H: ScriptingHost>(&self, limits: Limits) -> Result<Box<dyn Realm<H>>, ScriptError> {
        Ok(Box::new(QuickJsRealm::new(limits)?))
    }
}

/// The share of the heap limit that a realm still holds, after a failed
/// run and a collection, that makes it dead: what is held stays held,
/// and the next allocations would fail too.
const DEAD_HEAP: (usize, usize) = (3, 4);

/// A promise rejected with no handler yet, and the error it reports if
/// it still has none at the end of the checkpoint.
struct Rejection {
    promise: Persistent<Value<'static>>,
    error: ScriptError,
}

/// The slot a run moves the host into.
type Slot<H> = Rc<RefCell<Option<H>>>;

struct QuickJsRealm<H: ScriptingHost> {
    /// Saved values must go before the context and runtime they belong
    /// to; `Drop` empties this first.
    rejections: Rc<RefCell<Vec<Rejection>>>,
    /// The bootstrap's entry points, which [`Realm::enter`] calls (M26).
    entries: Option<Persistent<Object<'static>>>,
    host: Slot<H>,
    /// Interrupt checks taken in this run, and whether they ran out.
    steps: Rc<Cell<u32>>,
    exhausted: Rc<Cell<bool>>,
    /// The host's work in this run not yet charged as a whole step.
    work: Rc<Cell<u64>>,
    /// Whether the run under way reports muted errors.
    muted: Rc<Cell<bool>>,
    limits: Limits,
    dead: Option<Death>,
    context: Context,
    runtime: Runtime,
}

impl<H: ScriptingHost> QuickJsRealm<H> {
    fn new(limits: Limits) -> Result<QuickJsRealm<H>, ScriptError> {
        let failed = |e: rquickjs::Error| ScriptError {
            kind: ErrorKind::Heap,
            message: format!("InternalError: {e}"),
            url: String::new(),
            line: 0,
            column: 0,
        };
        let runtime = Runtime::new().map_err(failed)?;
        runtime.set_memory_limit(limits.heap);
        runtime.set_max_stack_size(limits.stack);
        let steps = Rc::new(Cell::new(0u32));
        let exhausted = Rc::new(Cell::new(false));
        let (taken, out, budget) = (steps.clone(), exhausted.clone(), limits.steps);
        runtime.set_interrupt_handler(Some(Box::new(move || {
            let n = taken.get().saturating_add(1);
            taken.set(n);
            if n > budget {
                out.set(true);
            }
            out.get()
        })));
        let rejections: Rc<RefCell<Vec<Rejection>>> = Rc::default();
        let muted = Rc::new(Cell::new(false));
        let (list, mute) = (rejections.clone(), muted.clone());
        runtime.set_host_promise_rejection_tracker(Some(Box::new(
            move |ctx: Ctx<'_>, promise: Value<'_>, reason: Value<'_>, handled: bool| {
                if handled {
                    list.borrow_mut().retain(|r| {
                        r.promise
                            .clone()
                            .restore(&ctx)
                            .map_or(true, |saved| saved != promise)
                    });
                } else {
                    let mut error = thrown(&ctx, reason, "", mute.get());
                    error.kind = ErrorKind::Rejection;
                    let promise = Persistent::save(&ctx, promise);
                    list.borrow_mut().push(Rejection { promise, error });
                }
            },
        )));
        let context = Context::full(&runtime).map_err(failed)?;
        let host: Slot<H> = Rc::default();
        let budget = dom::Budget {
            steps: steps.clone(),
            work: Rc::default(),
            exhausted: exhausted.clone(),
            limit: limits.steps,
        };
        let work = budget.work.clone();
        let entries = context
            .with(|ctx| {
                install(&ctx, &host, &budget)
                    .map(|entries| Persistent::save(&ctx, entries))
                    .catch(&ctx)
                    .map_err(|e| e.to_string())
            })
            .map_err(|message| ScriptError {
                kind: ErrorKind::Exception,
                message,
                url: String::new(),
                line: 0,
                column: 0,
            })?;
        Ok(QuickJsRealm {
            rejections,
            entries: Some(entries),
            host,
            steps,
            exhausted,
            work,
            muted,
            limits,
            dead: None,
            context,
            runtime,
        })
    }

    /// Moves `host` in and renews the budget.
    fn begin(&mut self, host: &mut H, muted: bool) {
        *self.host.borrow_mut() = Some(std::mem::take(host));
        self.steps.set(0);
        self.work.set(0);
        self.exhausted.set(false);
        self.muted.set(muted);
    }

    /// Moves the host back out.
    fn end(&mut self, host: &mut H) {
        if let Some(back) = self.host.borrow_mut().take() {
            *host = back;
        }
        self.muted.set(false);
    }

    /// Whether the heap, collected, is still nearly full: then the realm
    /// is dead.
    fn check_heap(&mut self) {
        self.runtime.run_gc();
        let (num, den) = DEAD_HEAP;
        if self.heap() > self.limits.heap / den * num {
            self.dead = Some(Death::Heap);
        }
    }

    /// `error` as a run's failure: the budget's when it ran out, the
    /// heap's when the realm is now dead.
    fn failure(&mut self, mut error: ScriptError) -> ScriptError {
        if self.exhausted.get() {
            error.kind = ErrorKind::Budget;
            error.message = "Script ran too long and was stopped".to_owned();
            return error;
        }
        self.check_heap();
        if self.dead.is_some() {
            error.kind = ErrorKind::Heap;
        }
        error
    }
}

impl<H: ScriptingHost> Drop for QuickJsRealm<H> {
    fn drop(&mut self) {
        self.runtime.set_host_promise_rejection_tracker(None);
        self.rejections.borrow_mut().clear();
        self.entries = None;
    }
}

impl<H: ScriptingHost> Realm<H> for QuickJsRealm<H> {
    fn run(&mut self, script: &Script<'_>, host: &mut H) -> Result<Completion, ScriptError> {
        if let Some(death) = self.dead {
            return Err(dead(death));
        }
        self.begin(host, script.muted);
        let result =
            self.context.with(
                |ctx| match evaluate(&ctx, script.source, script.url, script.line) {
                    Ok(value) => Ok(completion(&value)),
                    Err(caught) => Err(caught_error(&ctx, caught, script.url, script.muted)),
                },
            );
        self.end(host);
        // A run that spent its budget is stopped, though the script caught
        // a native's refusal and ended before the next interrupt check.
        let result = match result {
            Ok(_) if self.exhausted.get() => Err(ScriptError {
                kind: ErrorKind::Budget,
                message: String::new(),
                url: script.url.to_owned(),
                line: 0,
                column: 0,
            }),
            other => other,
        };
        result.map_err(|error| self.failure(error))
    }

    fn enter(&mut self, task: &Task, host: &mut H) -> Result<(), ScriptError> {
        if let Some(death) = self.dead {
            return Err(dead(death));
        }
        let Some(entries) = self.entries.clone() else {
            return Ok(());
        };
        self.begin(host, false);
        let result = self.context.with(|ctx| {
            let called = entries.restore(&ctx).and_then(|entries| match task {
                Task::Timer(id) => entries
                    .get::<_, Function<'_>>("timer")?
                    .call::<_, Value<'_>>((*id,)),
            });
            called
                .catch(&ctx)
                .map(|_| ())
                .map_err(|caught| caught_error(&ctx, caught, "", false))
        });
        self.end(host);
        let result = match result {
            Ok(()) if self.exhausted.get() => Err(ScriptError {
                kind: ErrorKind::Budget,
                message: String::new(),
                url: String::new(),
                line: 0,
                column: 0,
            }),
            other => other,
        };
        result.map_err(|error| self.failure(error))
    }

    fn checkpoint(&mut self, host: &mut H) -> Vec<ScriptError> {
        if let Some(death) = self.dead {
            return vec![dead(death)];
        }
        self.begin(host, false);
        let mut errors = Vec::new();
        while self.runtime.is_job_pending() && !self.exhausted.get() {
            match self.runtime.execute_pending_job() {
                Ok(_) => {}
                Err(job) => {
                    let error = job.0.with(|ctx| {
                        let thrown = ctx.catch();
                        thrown_or_error(&ctx, thrown)
                    });
                    errors.push(error);
                }
            }
        }
        self.end(host);
        let errors: Vec<ScriptError> = errors.into_iter().map(|e| self.failure(e)).collect();
        let mut reported: Vec<ScriptError> = errors;
        reported.extend(
            std::mem::take(&mut *self.rejections.borrow_mut())
                .into_iter()
                .map(|r| r.error),
        );
        reported
    }

    fn heap(&self) -> usize {
        usize::try_from(self.runtime.memory_usage().malloc_size).unwrap_or(usize::MAX)
    }

    fn collect(&mut self) {
        self.runtime.run_gc();
    }

    fn dead(&self) -> Option<Death> {
        self.dead
    }
}

/// Runs `source` as a classic script: global, not strict, its errors
/// naming `url`, its first line counted as `line` of its document (line
/// feeds go before the text, so that QuickJS-ng counts them).
fn evaluate<'js>(
    ctx: &Ctx<'js>,
    source: &str,
    url: &str,
    line: u32,
) -> Result<Value<'js>, CaughtError<'js>> {
    let skip = line.saturating_sub(1) as usize;
    let mut text = String::with_capacity(skip + source.len());
    text.extend(std::iter::repeat_n('\n', skip));
    text.push_str(source);
    let mut options = EvalOptions::default();
    options.global = true;
    options.strict = false;
    options.filename = Some(url.replace('\0', "%00"));
    ctx.eval_with_options::<Value<'_>, _>(text, options)
        .catch(ctx)
}

/// The error a dead realm reports for what it was asked to run.
fn dead(death: Death) -> ScriptError {
    let message = match death {
        Death::Heap => "Scripts stopped: out of memory",
    };
    ScriptError {
        kind: ErrorKind::Dead,
        message: message.to_owned(),
        url: String::new(),
        line: 0,
        column: 0,
    }
}

/// What a script's completion value is kept as.
fn completion(value: &Value<'_>) -> Completion {
    if value.is_undefined() {
        Completion::Undefined
    } else if let Some(text) = value.as_string() {
        text.to_string()
            .map(Completion::String)
            .unwrap_or(Completion::Other)
    } else {
        Completion::Other
    }
}

/// The error a caught failure reports.
fn caught_error<'js>(
    ctx: &Ctx<'js>,
    caught: CaughtError<'js>,
    url: &str,
    muted: bool,
) -> ScriptError {
    match caught {
        CaughtError::Exception(exception) => {
            let error = exception_error(&exception, url);
            mute(error, muted)
        }
        CaughtError::Value(value) => thrown(ctx, value, url, muted),
        CaughtError::Error(error) => mute(
            ScriptError {
                kind: ErrorKind::Exception,
                message: format!("InternalError: {error}"),
                url: url.to_owned(),
                line: 0,
                column: 0,
            },
            muted,
        ),
    }
}

/// The error a thrown value, or a failure with none, reports.
fn thrown_or_error<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> ScriptError {
    match as_exception(&value) {
        Some(exception) => exception_error(&exception, ""),
        None => thrown(ctx, value, "", false),
    }
}

/// The error a thrown value reports: an `Error`'s name, message and
/// position, else the value as a string.
fn thrown<'js>(ctx: &Ctx<'js>, value: Value<'js>, url: &str, muted: bool) -> ScriptError {
    if let Some(exception) = as_exception(&value) {
        return mute(exception_error(&exception, url), muted);
    }
    let message = console::describe(ctx, &value);
    mute(
        ScriptError {
            kind: ErrorKind::Exception,
            message,
            url: url.to_owned(),
            line: 0,
            column: 0,
        },
        muted,
    )
}

/// `value` as an `Error` object, if it is one.
fn as_exception<'js>(value: &Value<'js>) -> Option<Exception<'js>> {
    value.as_object().cloned().and_then(Exception::from_object)
}

/// An `Error` object's report: `name: message`, at the first frame of
/// its stack.
fn exception_error(exception: &Exception<'_>, url: &str) -> ScriptError {
    let name = exception
        .as_object()
        .get::<_, Coerced<String>>("name")
        .map(|name| name.0)
        .unwrap_or_else(|_| "Error".to_owned());
    let message = match exception.message() {
        Some(text) if !text.is_empty() => format!("{name}: {text}"),
        _ => name,
    };
    let (at, line, column) = exception
        .stack()
        .and_then(|stack| first_frame(&stack))
        .unwrap_or_else(|| (url.to_owned(), 0, 0));
    ScriptError {
        kind: ErrorKind::Exception,
        message,
        url: at,
        line,
        column,
    }
}

/// The URL, line and column of a stack's first frame:
/// `    at f (url:line:column)` or `    at url:line:column`. The URL may
/// hold spaces and parentheses, as a `data:` URL's may: the place is
/// what follows the name's ` (`.
fn first_frame(stack: &str) -> Option<(String, u32, u32)> {
    let frame = stack.lines().next()?.trim().strip_prefix("at ")?;
    let place = match frame.strip_suffix(')') {
        Some(called) => &called[called.find(" (")? + 2..],
        None => frame,
    };
    let mut parts = place.rsplitn(3, ':');
    let column = parts.next()?.parse().ok()?;
    let line = parts.next()?.parse().ok()?;
    let url = parts.next()?;
    Some((url.to_owned(), line, column))
}

/// `error`, muted if `muted` (HTML section 8.1.4.6): its details
/// hidden.
fn mute(error: ScriptError, muted: bool) -> ScriptError {
    if !muted {
        return error;
    }
    ScriptError {
        kind: error.kind,
        message: "Script error.".to_owned(),
        url: String::new(),
        line: 0,
        column: 0,
    }
}

/// Fills in a new realm's global object: `window` and `self`,
/// `console`, and the DOM's interfaces and `document` (M25); and takes
/// away what QuickJS-ng adds that a page of a browser that is not
/// cross-origin isolated has not: an unrounded `performance` clock and
/// `SharedArrayBuffer`.
fn install<'js, H: ScriptingHost>(
    ctx: &Ctx<'js>,
    host: &Slot<H>,
    budget: &dom::Budget,
) -> rquickjs::Result<Object<'js>> {
    let global = ctx.globals();
    global.remove("performance")?;
    global.remove("SharedArrayBuffer")?;
    global.set("window", global.clone())?;
    global.set("self", global.clone())?;
    let console = Object::new(ctx.clone())?;
    for (name, level) in [
        ("log", ConsoleLevel::Log),
        ("info", ConsoleLevel::Info),
        ("warn", ConsoleLevel::Warn),
        ("error", ConsoleLevel::Error),
        ("debug", ConsoleLevel::Debug),
    ] {
        let host = host.clone();
        let method = Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, args: Rest<Value<'js>>| -> rquickjs::Result<()> {
                // Formatting may run the page's code, so the host is
                // borrowed only once the message is made.
                let Some(message) = console::format(&ctx, &args.0)? else {
                    return Ok(());
                };
                with_host(&ctx, &host, |host| host.console(level, message))
            },
        )?
        .with_name(name)?;
        console.set(name, method)?;
    }
    global.set("console", console)?;
    dom::install(ctx, host, budget)
}

/// Calls `f` with the host, which is there only while a run the engine
/// started is under way; otherwise throws.
fn with_host<H: ScriptingHost, R>(
    ctx: &Ctx<'_>,
    host: &Slot<H>,
    f: impl FnOnce(&mut H) -> R,
) -> rquickjs::Result<R> {
    let Ok(mut slot) = host.try_borrow_mut() else {
        return Err(Exception::throw_internal(ctx, "the host is busy"));
    };
    match slot.as_mut() {
        Some(host) => Ok(f(host)),
        None => Err(Exception::throw_internal(ctx, "no script is running")),
    }
}
