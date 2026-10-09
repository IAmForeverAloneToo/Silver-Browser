use crate::test_tree::Tree;
use crate::{
    Answer, Call, Completion, ConsoleLevel, Death, DomError, ErrorKind, Limits, QuickJs, Realm,
    Script, ScriptEngine, ScriptError, ScriptingHost, Task,
};

/// A host that records what scripts do to it, with a small tree for the
/// bindings to reach.
#[derive(Debug, Default)]
pub(crate) struct Recorder {
    pub(crate) console: Vec<(ConsoleLevel, String)>,
    pub(crate) title: String,
    pub(crate) titles_read: usize,
    pub(crate) tree: Tree,
    /// The timers set, by id and delay, in order (M26), and those
    /// cleared.
    pub(crate) timers: Vec<(u32, i32)>,
    pub(crate) cleared: Vec<u32>,
    /// What `performance.now()` answers.
    pub(crate) now: f64,
    /// The errors reported to the host (M26): those of listeners,
    /// handlers and timers, which the bootstrap reports.
    pub(crate) reported: Vec<ScriptError>,
    /// The elements whose activation behaviour ran, and the dialogs
    /// shown, by kind and message.
    pub(crate) activated: Vec<u64>,
    pub(crate) dialogs: Vec<(String, String)>,
}

impl ScriptingHost for Recorder {
    fn console(&mut self, level: ConsoleLevel, message: String) {
        self.console.push((level, message));
    }

    fn title(&mut self) -> String {
        self.titles_read += 1;
        self.title.clone()
    }

    fn set_title(&mut self, title: String) {
        self.title = title;
    }

    fn dom(&mut self, call: Call) -> Result<Answer, DomError> {
        match call {
            Call::SetTimer { id, delay } => {
                self.timers.push((id, delay));
                Ok(Answer::Nothing)
            }
            Call::ClearTimer(id) => {
                self.cleared.push(id);
                Ok(Answer::Nothing)
            }
            Call::Now => Ok(Answer::Number(self.now)),
            Call::TimeOrigin => Ok(Answer::Number(1_700_000_000_000.5)),
            Call::DocumentUrl => Ok(Answer::MaybeString(Some(
                "https://example.com/page.html".to_owned(),
            ))),
            Call::Activate(handle) => {
                self.activated.push(handle.0);
                Ok(Answer::Nothing)
            }
            Call::Dialog { kind, message } => {
                self.dialogs.push((kind, message));
                Ok(Answer::Nothing)
            }
            call => self.tree.dom(call),
        }
    }

    fn take_work(&mut self) -> u64 {
        self.tree.take_work()
    }

    fn report(&mut self, error: ScriptError) {
        self.reported.push(error);
    }
}

pub(crate) fn realm_with(limits: Limits) -> Box<dyn Realm<Recorder>> {
    QuickJs.realm(limits).expect("a realm")
}

pub(crate) fn realm() -> Box<dyn Realm<Recorder>> {
    realm_with(Limits::default())
}

pub(crate) fn script(source: &str) -> Script<'_> {
    Script {
        source,
        url: "https://example.com/page.html",
        line: 1,
        muted: false,
    }
}

/// The console's lines after running `source` and a checkpoint.
fn logged(source: &str) -> Vec<String> {
    let mut realm = realm();
    let mut host = Recorder::default();
    realm.run(&script(source), &mut host).expect("it runs");
    assert!(realm.checkpoint(&mut host).is_empty());
    host.console.into_iter().map(|(_, text)| text).collect()
}

#[test]
fn a_script_runs_and_ends_with_its_completion_value() {
    let mut realm = realm();
    let mut host = Recorder::default();
    let run = |realm: &mut Box<dyn Realm<Recorder>>, host: &mut Recorder, source| {
        realm.run(&script(source), host).expect("it runs")
    };
    assert_eq!(
        run(&mut realm, &mut host, "1 + 1; undefined"),
        Completion::Undefined
    );
    assert_eq!(
        run(&mut realm, &mut host, "'a' + 'b'"),
        Completion::String("ab".to_owned())
    );
    assert_eq!(run(&mut realm, &mut host, "({})"), Completion::Other);
    // Scripts share their realm's global object, and are sloppy.
    run(&mut realm, &mut host, "var shared = 7; undeclared = 8;");
    assert_eq!(
        run(&mut realm, &mut host, "String(shared + undeclared)"),
        Completion::String("15".to_owned())
    );
    assert_eq!(
        run(
            &mut realm,
            &mut host,
            "String(window === globalThis && self === window)"
        ),
        Completion::String("true".to_owned())
    );
}

#[test]
fn the_console_formats_as_the_console_standard_says() {
    assert_eq!(logged("console.log('hello')"), ["hello"]);
    assert_eq!(
        logged("console.log('a', 1, true, null, undefined)"),
        ["a 1 true null undefined"]
    );
    assert_eq!(
        logged("console.log(1, 'a')"),
        ["1 a"],
        "a format only first"
    );
    assert_eq!(
        logged("console.log('%s is %d years, %f%%', 'Ann', '42.9px', '1.5e1x')"),
        ["Ann is 42 years, 15%"]
    );
    assert_eq!(logged("console.log('%i|%d', -3.7, 'x')"), ["-3|NaN"]);
    assert_eq!(
        logged("console.log('%c styled', 'color: red')"),
        [" styled"]
    );
    assert_eq!(
        logged("console.log('%s %s', 'only')"),
        ["only %s"],
        "too few arguments"
    );
    assert_eq!(
        logged("console.log('%s', 'a', 'b', 2)"),
        ["a b 2"],
        "too many"
    );
    assert_eq!(
        logged("console.log('%x %', 1)"),
        ["%x % 1"],
        "not a specifier"
    );
    assert!(
        logged("console.log()").is_empty(),
        "no arguments log nothing"
    );
    assert_eq!(
        logged("console.log(Symbol('s'), [1, 2])"),
        ["Symbol(s) 1,2"]
    );
    assert_eq!(logged("console.log('%d', Symbol('s'))"), ["NaN"]);
    assert_eq!(
        logged("console.log({ toString() { throw 1; } }, Object.create(null))"),
        ["[object] [object]"],
        "a value that cannot be a string"
    );
    // Each method its level.
    let mut realm = realm();
    let mut host = Recorder::default();
    realm
        .run(
            &script("console.info(1); console.warn(2); console.error(3); console.debug(4)"),
            &mut host,
        )
        .expect("it runs");
    let levels: Vec<ConsoleLevel> = host.console.iter().map(|(level, _)| *level).collect();
    assert_eq!(
        levels,
        [
            ConsoleLevel::Info,
            ConsoleLevel::Warn,
            ConsoleLevel::Error,
            ConsoleLevel::Debug
        ]
    );
}

#[test]
fn a_console_argument_that_logs_while_it_is_formatted_logs_first() {
    // Formatting runs the page's code; the host is not borrowed then.
    assert_eq!(
        logged("console.log({ toString() { console.log('inner'); return 'outer'; } })"),
        ["inner", "outer"]
    );
}

#[test]
fn parse_int_and_parse_float_are_ecmascripts() {
    use crate::quickjs::console::{parse_float, parse_int};
    assert_eq!(parse_int("  42px"), 42.0);
    assert_eq!(parse_int("-0x1"), -0.0);
    assert!(parse_int("px").is_nan());
    assert!(parse_int("").is_nan());
    assert_eq!(parse_int("\u{a0}+7"), 7.0);
    assert_eq!(parse_float("3.25e2abc"), 325.0);
    assert_eq!(parse_float(".5"), 0.5);
    assert_eq!(parse_float("-.5"), -0.5);
    assert_eq!(parse_float("5."), 5.0);
    assert_eq!(parse_float("1e"), 1.0);
    assert_eq!(parse_float("-Infinityx"), f64::NEG_INFINITY);
    assert!(parse_float(".").is_nan());
    assert!(parse_float("e5").is_nan());
}

#[test]
fn document_title_reads_and_writes_through_the_host() {
    let mut realm = realm();
    let mut host = Recorder {
        title: "Before".to_owned(),
        ..Recorder::default()
    };
    let done = realm
        .run(
            &script("var was = document.title; document.title = 42; was + '|' + document.title"),
            &mut host,
        )
        .expect("it runs");
    assert_eq!(done, Completion::String("Before|42".to_owned()));
    assert_eq!(host.title, "42");
    assert_eq!(host.titles_read, 2);
    // A value that cannot be a string throws, and the title stays.
    let error = realm
        .run(&script("document.title = Object.create(null)"), &mut host)
        .expect_err("it throws");
    assert!(error.message.starts_with("TypeError"), "{error:?}");
    assert_eq!(host.title, "42");
}

#[test]
fn the_host_is_the_runs_alone() {
    let mut realm = realm();
    let mut host = Recorder::default();
    // A function kept from one run calls the host of the next.
    realm
        .run(&script("var later = () => console.log('later')"), &mut host)
        .expect("it runs");
    let mut other = Recorder::default();
    realm.run(&script("later()"), &mut other).expect("it runs");
    assert!(host.console.is_empty());
    assert_eq!(other.console.len(), 1);
}

#[test]
fn errors_name_their_kind_message_and_place() {
    let mut realm = realm();
    let mut host = Recorder::default();
    let inline = Script {
        source: "\nlet x = 1;\nmissing();",
        url: "https://example.com/page.html",
        line: 10,
        muted: false,
    };
    let error = realm.run(&inline, &mut host).expect_err("it throws");
    assert_eq!(error.kind, ErrorKind::Exception);
    assert_eq!(error.message, "ReferenceError: missing is not defined");
    assert_eq!(error.url, "https://example.com/page.html");
    assert_eq!((error.line, error.column), (12, 1), "the document's line");
    let syntax = realm
        .run(&script("let y = ;"), &mut host)
        .expect_err("a syntax error");
    assert!(syntax.message.starts_with("SyntaxError"), "{syntax:?}");
    assert_eq!(syntax.line, 1);
    let value = realm
        .run(&script("throw 'just a string'"), &mut host)
        .expect_err("it throws");
    assert_eq!(value.message, "just a string");
    // A URL with spaces and parentheses in it, as a `data:` URL may have.
    let data = Script {
        source: "throw new Error('x')",
        url: "data:text/javascript,throw new Error('x') (a)",
        line: 1,
        muted: false,
    };
    let error = realm.run(&data, &mut host).expect_err("it throws");
    assert_eq!(error.url, data.url);
    assert_eq!((error.line, error.column), (1, 11));
    let called = Script {
        source: "function f() { null.x }\nf()",
        ..data
    };
    let error = realm.run(&called, &mut host).expect_err("it throws");
    assert_eq!((error.url.as_str(), error.line), (data.url, 1));
    // An error in one script leaves the realm to the next.
    assert!(realm.run(&script("1"), &mut host).is_ok());
}

#[test]
fn a_muted_scripts_errors_say_nothing_of_it() {
    let mut realm = realm();
    let mut host = Recorder::default();
    let muted = Script {
        source: "secret_value_42",
        url: "https://other.example/data.js",
        line: 1,
        muted: true,
    };
    let error = realm.run(&muted, &mut host).expect_err("it throws");
    assert_eq!(error.message, "Script error.");
    assert_eq!((error.url.as_str(), error.line, error.column), ("", 0, 0));
}

#[test]
fn promise_jobs_run_at_the_checkpoint_and_unhandled_rejections_are_reported() {
    let mut realm = realm();
    let mut host = Recorder::default();
    realm
        .run(
            &script(
                "Promise.resolve().then(() => console.log('job')); \
                 console.log('script'); \
                 Promise.reject(new TypeError('lost')); \
                 Promise.reject(new Error('caught later')).catch(() => {});",
            ),
            &mut host,
        )
        .expect("it runs");
    assert_eq!(host.console.len(), 1, "jobs wait for the checkpoint");
    let errors = realm.checkpoint(&mut host);
    assert_eq!(host.console[1].1, "job");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].kind, ErrorKind::Rejection);
    assert_eq!(errors[0].message, "TypeError: lost");
    // An error thrown by a job is reported too.
    realm
        .run(
            &script("Promise.resolve().then(() => { throw new RangeError('in a job'); })"),
            &mut host,
        )
        .expect("it runs");
    let errors = realm.checkpoint(&mut host);
    assert!(
        errors.iter().any(|e| e.message == "RangeError: in a job"),
        "{errors:?}"
    );
}

#[test]
fn a_script_past_its_step_budget_is_stopped_and_the_realm_goes_on() {
    let mut realm = realm_with(Limits {
        steps: 20,
        ..Limits::default()
    });
    let mut host = Recorder::default();
    let error = realm
        .run(&script("for (;;) {}"), &mut host)
        .expect_err("stopped");
    assert_eq!(error.kind, ErrorKind::Budget);
    // Uncatchable, even inside a native function's callback.
    let error = realm
        .run(
            &script("try { [3, 2, 1].sort(() => { for (;;) {} }); } catch (e) { console.log('caught'); }"),
            &mut host,
        )
        .expect_err("stopped");
    assert_eq!(error.kind, ErrorKind::Budget);
    assert!(host.console.is_empty());
    // A checkpoint's jobs have a budget of their own.
    realm
        .run(
            &script("Promise.resolve().then(() => { for (;;) {} })"),
            &mut host,
        )
        .expect("it runs");
    let errors = realm.checkpoint(&mut host);
    assert_eq!(errors[0].kind, ErrorKind::Budget);
    // The next runs have budgets of their own: each of these takes about
    // half of one.
    let half = "{ let n = 0; for (let i = 0; i < 50000; i++) n += i; } 'still here'";
    for _ in 0..3 {
        assert_eq!(
            realm.run(&script(half), &mut host),
            Ok(Completion::String("still here".to_owned()))
        );
    }
    assert_eq!(realm.dead(), None);
}

#[test]
fn a_script_that_runs_out_of_heap_kills_the_realm_only_if_it_keeps_it() {
    let limits = Limits {
        heap: 8 << 20,
        ..Limits::default()
    };
    let grow = "for (let i = 0; ; i++) kept.push('x'.repeat(1000) + i);";
    // What a script lets go of comes back: the realm lives.
    let mut realm = realm_with(limits);
    let mut host = Recorder::default();
    let error = realm
        .run(
            &script(&format!("(() => {{ let kept = []; {grow} }})()")),
            &mut host,
        )
        .expect_err("out of memory");
    assert_eq!(error.kind, ErrorKind::Exception, "{error:?}");
    assert_eq!(realm.dead(), None);
    assert!(realm.run(&script("1"), &mut host).is_ok());
    // Caught, with the memory let go: the realm lives. What is caught is
    // QuickJS-ng's `InternalError`, or `null` when even that could not
    // be made.
    let caught = format!("try {{ let kept = []; {grow} }} catch (e) {{ console.log('caught'); }}");
    realm.run(&script(&caught), &mut host).expect("caught");
    assert_eq!(host.console[0].1, "caught");
    assert_eq!(realm.dead(), None);
    // Held by a global: the realm is dead, and runs nothing more.
    let error = realm
        .run(&script(&format!("var kept = []; {grow}")), &mut host)
        .expect_err("out of memory");
    assert_eq!(error.kind, ErrorKind::Heap);
    assert_eq!(realm.dead(), Some(Death::Heap));
    let error = realm.run(&script("1"), &mut host).expect_err("dead");
    assert_eq!(error.kind, ErrorKind::Dead);
    assert_eq!(realm.checkpoint(&mut host)[0].kind, ErrorKind::Dead);
    assert!(realm.heap() > 6 << 20);
}

#[test]
fn deep_recursion_throws_a_catchable_range_error() {
    let done = realm()
        .run(
            &script("function f() { return f(); } try { f(); 'no' } catch (e) { e.name + ': ' + e.message }"),
            &mut Recorder::default(),
        )
        .expect("it runs");
    assert_eq!(
        done,
        Completion::String("RangeError: Maximum call stack size exceeded".to_owned())
    );
}

#[test]
fn a_realm_has_no_unrounded_clock_and_no_shared_memory() {
    let done = realm()
        .run(
            &script(
                "[typeof SharedArrayBuffer, typeof Atomics, \
                 Object.prototype.toString.call(performance), performance.now(), \
                 performance.timeOrigin, JSON.stringify(performance), \
                 performance instanceof EventTarget].join()",
            ),
            &mut Recorder {
                now: 12.3,
                ..Recorder::default()
            },
        )
        .expect("it runs");
    // `performance` is the host's clock (M26), not QuickJS-ng's unrounded
    // one.
    assert_eq!(
        done,
        Completion::String(
            "undefined,object,[object Performance],12.3,1700000000000.5,\
             {\"timeOrigin\":1700000000000.5},true"
                .to_owned()
        )
    );
}

#[test]
fn timers_keep_their_callbacks_and_the_host_their_times() {
    let mut realm = realm();
    let mut host = Recorder::default();
    let done = realm
        .run(
            &script(
                "var log = []; \
                 var a = setTimeout(function (x, y) { 'use strict'; log.push('a' + x + y + (this === window)); }, 10, 1, 2); \
                 var b = setInterval(function () { log.push('b'); if (log.length > 3) clearInterval(b); }); \
                 var c = setTimeout(\"log.push('c' + typeof b)\", -5.7); \
                 clearTimeout(99); \
                 [a, b, c, setTimeout.length, typeof window.setInterval].join()",
            ),
            &mut host,
        )
        .expect("it runs");
    assert_eq!(done, Completion::String("1,2,3,1,function".to_owned()));
    assert_eq!(host.timers, [(1, 10), (2, 0), (3, -5)]);
    assert!(host.cleared.is_empty());
    let mut enter = |id: u32, host: &mut Recorder| {
        realm.enter(&Task::Timer(id), host).expect("it runs");
    };
    // A timeout runs once; a string is a classic script; an interval is
    // set again after each run until its callback clears it.
    enter(1, &mut host);
    enter(1, &mut host);
    enter(3, &mut host);
    enter(2, &mut host);
    enter(2, &mut host);
    assert_eq!(host.reported, []);
    assert_eq!(host.timers, [(1, 10), (2, 0), (3, -5), (2, 0)]);
    assert_eq!(host.cleared, [2]);
    enter(2, &mut host);
    let log = realm
        .run(&script("log.join()"), &mut host)
        .expect("it runs");
    assert_eq!(log, Completion::String("a12true,cnumber,b,b".to_owned()));
    // Ids are not given twice while their timers are set.
    let ids = realm
        .run(
            &script("[setTimeout(function () {}), setTimeout(function () {})].join()"),
            &mut host,
        )
        .expect("it runs");
    assert_eq!(ids, Completion::String("4,5".to_owned()));
}

#[test]
fn a_timer_that_throws_or_runs_too_long_fails_alone() {
    let mut realm = realm_with(Limits {
        steps: 20,
        ..Limits::default()
    });
    let mut host = Recorder::default();
    realm
        .run(
            &script(
                "setTimeout(function () {\n  throw new Error('boom');\n}); \
                 setTimeout(function () { for (;;) {} }); \
                 setInterval(function () { document.title = 'ran'; }, 5);",
            ),
            &mut host,
        )
        .expect("it runs");
    // What a timer's callback throws is reported, as an uncaught
    // exception is, and the task goes on.
    realm.enter(&Task::Timer(1), &mut host).expect("it runs");
    let thrown = host.reported.pop().expect("it was reported");
    assert_eq!(thrown.kind, ErrorKind::Exception);
    assert_eq!(thrown.message, "Error: boom");
    assert_eq!(
        (thrown.url.as_str(), thrown.line),
        ("https://example.com/page.html", 2)
    );
    let stopped = realm
        .enter(&Task::Timer(2), &mut host)
        .expect_err("it is stopped");
    assert_eq!(stopped.kind, ErrorKind::Budget);
    realm.enter(&Task::Timer(3), &mut host).expect("it runs");
    assert_eq!(host.title, "ran");
    assert_eq!(host.timers.last(), Some(&(3, 5)));
    let none = realm
        .run(&script("setTimeout()"), &mut host)
        .expect_err("it throws");
    assert_eq!(
        none.message,
        "TypeError: Failed to execute 'setTimeout' on 'Window': 1 argument required, but only 0 present."
    );
}

#[test]
fn a_realm_reports_its_heap_and_collects() {
    let mut realm = realm();
    let mut host = Recorder::default();
    let before = realm.heap();
    realm
        .run(&script("var big = 'x'.repeat(1 << 20) + 'y';"), &mut host)
        .expect("it runs");
    assert!(realm.heap() > before + (1 << 20));
    realm
        .run(&script("big = null"), &mut host)
        .expect("it runs");
    realm.collect();
    assert!(realm.heap() < before + (1 << 19));
}
