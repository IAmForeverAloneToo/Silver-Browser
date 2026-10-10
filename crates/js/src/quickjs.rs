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
    Completion, ConsoleLevel, Death, ErrorKind, FetchAnswer, Field, Limits, Realm, Script,
    ScriptEngine, ScriptError, ScriptingHost, Target, Task,
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

/// The bootstrap's entry points (M26), which the realm's tasks and the
/// error reports of scripts run nested call; emptied before the
/// context goes.
type Entries = Rc<RefCell<Option<Persistent<Object<'static>>>>>;

struct QuickJsRealm<H: ScriptingHost> {
    /// Saved values must go before the context and runtime they belong
    /// to; `Drop` empties this first.
    rejections: Rc<RefCell<Vec<Rejection>>>,
    /// The bootstrap's entry points, which [`Realm::enter`] calls (M26).
    entries: Entries,
    /// The errors of the jobs run between an engine dispatch's
    /// listeners, which the next checkpoint reports.
    deferred: Vec<ScriptError>,
    /// The last run spent its budget, perhaps inside the bootstrap: the
    /// next one first puts the bootstrap's own state back.
    unwind: bool,
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
        let entries: Entries = Rc::default();
        let saved = context
            .with(|ctx| {
                install(&ctx, &host, &budget, &entries)
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
        *entries.borrow_mut() = Some(saved);
        Ok(QuickJsRealm {
            rejections,
            entries,
            deferred: Vec::new(),
            unwind: false,
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

    /// Moves `host` in and renews the budget; after a run its budget
    /// stopped, the bootstrap's own state is put back first (M26).
    fn begin(&mut self, host: &mut H, muted: bool) {
        *self.host.borrow_mut() = Some(std::mem::take(host));
        self.steps.set(0);
        self.work.set(0);
        self.exhausted.set(false);
        self.muted.set(muted);
        if std::mem::take(&mut self.unwind)
            && let Some(entries) = self.entries.borrow().clone()
        {
            self.context.with(|ctx| {
                let reset = entries.restore(&ctx).and_then(|entries| {
                    entries
                        .get::<_, Function<'_>>("reset")?
                        .call::<_, Value<'_>>(())
                });
                let _ = reset.catch(&ctx);
            });
        }
    }

    /// Moves the host back out.
    fn end(&mut self, host: &mut H) {
        if let Some(back) = self.host.borrow_mut().take() {
            *host = back;
        }
        self.muted.set(false);
        if self.exhausted.get() {
            self.unwind = true;
        }
    }

    /// Runs the promise jobs queued while the budget lasts; the errors
    /// of those that threw.
    fn jobs(&mut self) -> Vec<ScriptError> {
        let mut errors = Vec::new();
        while self.runtime.is_job_pending() && !self.exhausted.get() {
            if let Err(job) = self.runtime.execute_pending_job() {
                let error = job.0.with(|ctx| {
                    let thrown = ctx.catch();
                    thrown_or_error(&ctx, thrown)
                });
                errors.push(error);
            }
        }
        errors
    }

    /// Runs one of the bootstrap's stepped tasks (M26): `start` calls the
    /// entry point that makes it, a generator, which is resumed until it
    /// ends, the microtask checkpoint run between its steps, the
    /// listeners or callbacks it calls. One budget covers it all. What it
    /// ends with: for a dispatch, whether the event was canceled.
    fn stepped<F>(
        &mut self,
        entries: &Persistent<Object<'static>>,
        start: F,
    ) -> Result<bool, ScriptError>
    where
        F: for<'js> FnOnce(&Ctx<'js>, Object<'js>) -> rquickjs::Result<Value<'js>>,
    {
        let run = self.context.with(|ctx| {
            let started = entries
                .clone()
                .restore(&ctx)
                .and_then(|entries| start(&ctx, entries));
            started
                .catch(&ctx)
                .map(|run| Persistent::save(&ctx, run))
                .map_err(|caught| caught_error(&ctx, caught, "", false))
        })?;
        loop {
            let step = self.context.with(|ctx| {
                let resumed = entries.clone().restore(&ctx).and_then(|entries| {
                    let run = run.clone().restore(&ctx)?;
                    entries
                        .get::<_, Function<'_>>("resume")?
                        .call::<_, Value<'_>>((run,))
                });
                resumed
                    .catch(&ctx)
                    .map(|value| value.as_bool())
                    .map_err(|caught| caught_error(&ctx, caught, "", false))
            })?;
            if let Some(canceled) = step {
                return Ok(canceled);
            }
            let errors = self.jobs();
            self.deferred.extend(errors);
        }
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
        *self.entries.borrow_mut() = None;
    }
}

impl<H: ScriptingHost> Realm<H> for QuickJsRealm<H> {
    fn run(&mut self, script: &Script<'_>, host: &mut H) -> Result<Completion, ScriptError> {
        if let Some(death) = self.dead {
            return Err(dead(death));
        }
        self.begin(host, script.muted);
        let (entries, exhausted) = (self.entries.clone(), self.exhausted.clone());
        let result =
            self.context.with(
                |ctx| match evaluate(&ctx, script.source, script.url, script.line) {
                    Ok(value) => Ok(completion(&value)),
                    Err(caught) => Err(report(
                        &ctx,
                        &entries,
                        &exhausted,
                        caught,
                        script.url,
                        script.muted,
                    )),
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

    fn enter(&mut self, task: &Task, host: &mut H) -> Result<bool, ScriptError> {
        if let Some(death) = self.dead {
            return Err(dead(death));
        }
        let Some(entries) = self.entries.borrow().clone() else {
            return Ok(false);
        };
        self.begin(host, false);
        let result = match task {
            Task::Timer(id) => self.context.with(|ctx| {
                let called = entries.clone().restore(&ctx).and_then(|entries| {
                    entries
                        .get::<_, Function<'_>>("timer")?
                        .call::<_, Value<'_>>((*id,))
                });
                called
                    .catch(&ctx)
                    .map(|_| false)
                    .map_err(|caught| caught_error(&ctx, caught, "", false))
            }),
            Task::Dispatch { target, event } => self.stepped(&entries, |ctx, entries| {
                let at = match target {
                    Target::Node(handle) => Value::new_float(ctx.clone(), handle.0 as f64),
                    Target::Window => Value::new_null(ctx.clone()),
                };
                let init = init_object(ctx, &event.fields)?;
                entries
                    .get::<_, Function<'_>>("dispatch")?
                    .call::<_, Value<'_>>((at, event.interface, event.kind.as_str(), init))
            }),
            Task::AnimationFrames(now) => self.stepped(&entries, |_, entries| {
                entries
                    .get::<_, Function<'_>>("frames")?
                    .call::<_, Value<'_>>((*now,))
            }),
            Task::Fetched { id, answer } => self.context.with(|ctx| {
                let called = entries.clone().restore(&ctx).and_then(|entries| {
                    let answer = match answer {
                        None => Value::new_null(ctx.clone()),
                        Some(answer) => answer_object(&ctx, answer)?.into_value(),
                    };
                    entries
                        .get::<_, Function<'_>>("fetched")?
                        .call::<_, Value<'_>>((*id, answer))
                });
                called
                    .catch(&ctx)
                    .map(|_| false)
                    .map_err(|caught| caught_error(&ctx, caught, "", false))
            }),
        };
        self.end(host);
        let result = match result {
            Ok(_) if self.exhausted.get() => Err(ScriptError {
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
        let mut errors = std::mem::take(&mut self.deferred);
        errors.extend(self.jobs());
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

/// Reports a script's uncaught exception as HTML section 8.1.4.6 does
/// (M26): an `error` event at the window, which the bootstrap fires,
/// and, unless a listener canceled it, the error the caller gives the
/// console, which is [`ErrorKind::Handled`] otherwise. A run its budget
/// stopped fires nothing.
fn report<'js>(
    ctx: &Ctx<'js>,
    entries: &Entries,
    exhausted: &Cell<bool>,
    caught: CaughtError<'js>,
    url: &str,
    muted: bool,
) -> ScriptError {
    let value = match &caught {
        CaughtError::Exception(exception) => exception.clone().into_value(),
        CaughtError::Value(value) => value.clone(),
        CaughtError::Error(_) => Value::new_undefined(ctx.clone()),
    };
    let mut error = caught_error(ctx, caught, url, muted);
    if exhausted.get() || error.kind != ErrorKind::Exception {
        return error;
    }
    let Some(entries) = entries.borrow().clone() else {
        return error;
    };
    let canceled = entries.restore(ctx).and_then(|entries| {
        entries.get::<_, Function<'_>>("report")?.call::<_, bool>((
            value,
            error.message.as_str(),
            error.url.as_str(),
            error.line,
            error.column,
            muted,
        ))
    });
    if let Ok(true) = canceled.catch(ctx) {
        error.kind = ErrorKind::Handled;
    }
    error
}

/// An event's dictionary as the bootstrap's constructors read it: an
/// object with no prototype, so that no getter a page put on
/// `Object.prototype` answers for a member it leaves out.
fn init_object<'js>(
    ctx: &Ctx<'js>,
    fields: &[(&'static str, Field)],
) -> rquickjs::Result<Object<'js>> {
    let object = Object::new(ctx.clone())?;
    object.set_prototype(None)?;
    for (name, field) in fields {
        let value = match field {
            Field::Bool(value) => Value::new_bool(ctx.clone(), *value),
            Field::Number(value) => Value::new_number(ctx.clone(), *value),
            Field::String(text) => rquickjs::String::from_str(ctx.clone(), text)?.into_value(),
            Field::Node(Some(handle)) => Value::new_float(ctx.clone(), handle.0 as f64),
            Field::Node(None) | Field::Null => Value::new_null(ctx.clone()),
            Field::Window => ctx.globals().into_value(),
            // The bootstrap's `dispatch` makes it the window's area.
            Field::Storage(local) => Value::new_bool(ctx.clone(), *local),
        };
        object.set(*name, value)?;
    }
    Ok(object)
}

/// A fetch's answer as the bootstrap's `fetched` takes it (M27.1): its
/// type, URL list, status, status text, fields as names and values one
/// after the other, and body, a buffer or `null`.
pub(crate) fn answer_object<'js>(
    ctx: &Ctx<'js>,
    answer: &FetchAnswer,
) -> rquickjs::Result<Object<'js>> {
    let object = Object::new(ctx.clone())?;
    object.set("type", answer.response_type)?;
    let urls = rquickjs::Array::new(ctx.clone())?;
    for (i, url) in answer.url_list.iter().enumerate() {
        urls.set(i, url.as_str())?;
    }
    object.set("urlList", urls)?;
    object.set("status", answer.status)?;
    object.set("statusText", answer.status_text.as_str())?;
    let headers = rquickjs::Array::new(ctx.clone())?;
    for (i, (name, value)) in answer.headers.iter().enumerate() {
        headers.set(2 * i, name.as_str())?;
        headers.set(2 * i + 1, value.as_str())?;
    }
    object.set("headers", headers)?;
    match &answer.body {
        Some(body) => object.set("body", rquickjs::ArrayBuffer::new_copy(ctx.clone(), body)?)?,
        None => object.set("body", Value::new_null(ctx.clone()))?,
    }
    Ok(object)
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
    entries: &Entries,
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
    dom::install(ctx, host, budget, entries)
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
