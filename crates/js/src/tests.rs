use crate::{
    Completion, ConsoleLevel, Death, ErrorKind, Limits, QuickJs, Realm, Script, ScriptEngine,
    ScriptingHost,
};

/// A host that records what scripts do to it.
#[derive(Debug, Default)]
struct Recorder {
    console: Vec<(ConsoleLevel, String)>,
    title: String,
    titles_read: usize,
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
}

fn realm_with(limits: Limits) -> Box<dyn Realm<Recorder>> {
    QuickJs.realm(limits).expect("a realm")
}

fn realm() -> Box<dyn Realm<Recorder>> {
    realm_with(Limits::default())
}

fn script(source: &str) -> Script<'_> {
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
            &script("[typeof performance, typeof SharedArrayBuffer, typeof Atomics].join()"),
            &mut Recorder::default(),
        )
        .expect("it runs");
    assert_eq!(
        done,
        Completion::String("undefined,undefined,object".to_owned())
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
