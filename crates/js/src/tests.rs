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
    /// shown, by kind, message and default: a confirm is answered OK
    /// unless it says "no", a prompt its default and a "!".
    pub(crate) activated: Vec<u64>,
    pub(crate) dialogs: Vec<(String, String, String)>,
    /// How often a frame was wanted, and whether the document is hidden.
    pub(crate) frames_wanted: usize,
    pub(crate) hidden: bool,
    /// The messages posted to the window (M31), by target origin.
    pub(crate) posted: Vec<(String, String)>,
    /// The `import()`s that wanted a module fetched (D61), by id, and
    /// the fetch options each asked with.
    pub(crate) imports: Vec<(u32, String)>,
    pub(crate) import_options: Vec<u32>,
    /// The frame its document is of and the frames under it, by their
    /// numbers, when it answers for frames (G90's tests).
    pub(crate) frames: Option<(u32, Vec<u32>)>,
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
            // A navigation whose response has begun, no later step come.
            Call::RandomBytes(count) => Ok(Answer::Bytes(
                (0..count).map(|i| (i * 37 + 11) as u8).collect(),
            )),
            Call::NavigationTiming => Ok(Answer::Numbers(vec![
                0.0, 200.0, 1.5, -1.0, -1.0, -1.0, -1.0, -1.0, -1.0, -1.0,
            ])),
            Call::DocumentUrl => Ok(Answer::MaybeString(Some(
                "https://example.com/page.html".to_owned(),
            ))),
            Call::Activate(handle) => {
                self.activated.push(handle.0);
                Ok(Answer::Nothing)
            }
            Call::Dialog {
                kind,
                message,
                default,
            } => {
                let answer = match kind.as_str() {
                    "confirm" if message == "no" => None,
                    "confirm" => Some("any string is OK".to_owned()),
                    "prompt" => Some(format!("{default}!")),
                    _ => None,
                };
                self.dialogs.push((kind, message, default));
                Ok(Answer::MaybeString(answer))
            }
            // A document with no policy compiles any code (M33.1).
            Call::CodeAllowed { .. } => Ok(Answer::String(String::new())),
            Call::CssomStyled(_) => Ok(Answer::Nothing),
            Call::StyleAllowed(_) => Ok(Answer::Bool(true)),
            Call::WantFrame => {
                self.frames_wanted += 1;
                Ok(Answer::Nothing)
            }
            Call::Hidden => Ok(Answer::Bool(self.hidden)),
            Call::Fonts(op, data) => Ok(match op.split(' ').next() {
                Some("status") => Answer::String("loaded".into()),
                // An sfnt's or a WOFF file's signature, for these tests.
                Some("valid" | "add") => {
                    Answer::Bool(data.starts_with(&[0, 1, 0, 0]) || data.starts_with(b"wOFF"))
                }
                _ => Answer::Nothing,
            }),
            Call::TargetOrigin(target) => Ok(Answer::String(match target.as_str() {
                "*" => target,
                _ => "https://example.com".to_owned(),
            })),
            Call::PostMessage {
                target_origin,
                data,
            } => {
                self.posted.push((target_origin, data));
                Ok(Answer::Nothing)
            }
            Call::ResolveModule { base, specifier } => resolve_module(&base, &specifier),
            Call::ImportModule { id, url, options } => {
                self.imports.push((id, url));
                self.import_options.push(options);
                Ok(Answer::Nothing)
            }
            Call::FrameSelf if self.frames.is_some() => Ok(Answer::Number(f64::from(
                self.frames.as_ref().map_or(0, |(me, _)| *me),
            ))),
            Call::FrameLength(_) if self.frames.is_some() => Ok(Answer::Number(
                self.frames.as_ref().map_or(0, |(_, under)| under.len()) as f64,
            )),
            Call::FrameChild(_, at) if self.frames.is_some() => Ok(Answer::Number(
                self.frames
                    .as_ref()
                    .and_then(|(_, under)| under.get(at as usize))
                    .map_or(-1.0, |&frame| f64::from(frame)),
            )),
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

/// A module specifier resolved as these tests need it (D61): a URL as it
/// is, a path against `base`'s directory (the page's when empty), a bare
/// name refused.
fn resolve_module(base: &str, specifier: &str) -> Result<Answer, DomError> {
    let base = match base {
        "" => "https://example.com/page.html",
        base => base,
    };
    if specifier.starts_with("https:") {
        return Ok(Answer::String(specifier.to_owned()));
    }
    let Some(path) = specifier.strip_prefix("./") else {
        return Err(DomError::new(
            "TypeError",
            format!(
                "Failed to resolve module specifier \"{specifier}\". Relative references \
                 must start with either \"/\", \"./\", or \"../\"."
            ),
        ));
    };
    let directory = base
        .rsplit_once('/')
        .map_or(base, |(directory, _)| directory);
    Ok(Answer::String(format!("{directory}/{path}")))
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
        base: "",
        options: 0,
    }
}

/// The console's lines after running `source` and a checkpoint.
pub(crate) fn logged(source: &str) -> Vec<String> {
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
        base: "",
        options: 0,
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
        base: "",
        options: 0,
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
        base: "",
        options: 0,
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
    // be made. The run ends well only if the catch caught it. (The
    // console, the bindings' own code since M36, may find no memory
    // inside the catch, so the test asks nothing of it there.)
    let caught = format!("try {{ let kept = []; {grow} }} catch (e) {{}}");
    realm.run(&script(&caught), &mut host).expect("caught");
    realm
        .run(&script("console.log('after')"), &mut host)
        .expect("logged");
    assert_eq!(host.console[0].1, "after");
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
fn a_page_recurses_three_thousand_deep_and_no_deeper_than_its_limit() {
    // G253: the limit holds (rquickjs takes one past 16 MiB for none, and
    // a script's recursion would end the process), and is deep enough
    // for what pages do: some 4,700 frames here, 3,000 on CI's Windows,
    // whose C compiler makes QuickJS's frame larger, 15,000 on macOS.
    let done = realm()
        .run(
            &script(
                "function f(n) { return n === 0 ? 0 : 1 + f(n - 1); }                  let deepest = 0;                  try { for (let n = 1000; ; n += 1000) { f(n); deepest = n; } }                  catch (e) { deepest + ' ' + e.name }",
            ),
            &mut Recorder::default(),
        )
        .expect("it runs");
    let Completion::String(text) = done else {
        panic!("{done:?}");
    };
    let (deepest, name) = text.split_once(' ').expect("a depth and a name");
    let deepest: u32 = deepest.parse().expect("a depth");
    assert_eq!(name, "RangeError");
    assert!(deepest >= 3_000, "{deepest} frames deep");
}

#[test]
fn no_realm_is_made_on_a_thread_without_room_for_its_scripts() {
    // G253: a thread of 2 MiB, where a page's recursion would end the
    // process, is refused a realm, and says which thread it was.
    let refused = std::thread::Builder::new()
        .name("small".to_owned())
        .stack_size(2 << 20)
        .spawn(|| {
            QuickJs
                .realm_of::<Recorder>(crate::RealmKind::Window, Limits::default(), None)
                .err()
        })
        .unwrap()
        .join()
        .unwrap()
        .expect("refused");
    assert_eq!(refused.kind, ErrorKind::Stack);
    assert!(refused.message.contains("\"small\""), "{}", refused.message);
}

#[test]
fn a_realm_has_no_unrounded_clock_and_no_shared_memory() {
    let done = realm()
        .run(
            &script(
                "[typeof SharedArrayBuffer, typeof Atomics, \
                 Object.prototype.toString.call(performance), performance.now(), \
                 performance.timeOrigin, Object.keys(performance.toJSON()).join(' '), \
                 performance.timing.responseStart - performance.timing.navigationStart, \
                 performance.timing.responseEnd, \
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
             timeOrigin timing navigation,1,0,true"
                .to_owned()
        )
    );
}

#[test]
fn the_date_reads_the_system_clock_to_the_millisecond() {
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("after the epoch")
        .as_millis();
    // The first step of the clock: a millisecond or so, not the coarse
    // clock's sixteen.
    let done = realm()
        .run(
            &script(
                "const a = Date.now(); let b = a; while (b === a) b = Date.now();                  const c = new Date().getTime(), d = Date.parse(Date());                  class Later extends Date {}                  const later = new Later(5);                  [Number.isInteger(a), b - a < 8, c >= b && c - b < 1000, d <= c && c - d < 2000,                  later instanceof Date && later.getTime() === 5 && new Date(7).getTime() === 7,                  new Date(2020, 1).getMonth() === 1 && Date.prototype.constructor === Date,                  a, Date.now.name, Date.now.length, String(Date.now), Date.name, Date.length,                  String(Date)].join()",
            ),
            &mut Recorder::default(),
        )
        .expect("it runs");
    let Completion::String(text) = done else {
        panic!("a string: {done:?}");
    };
    let parts: Vec<&str> = text.split(',').collect();
    assert_eq!(parts[..6], ["true"; 6], "{text}");
    let read: u128 = parts[6].parse().expect("a number");
    // Not before the time read just before: QuickJS's coarse clock may
    // read up to a step behind.
    assert!(read >= before && read < before + 5_000, "{read} {before}");
    assert_eq!(
        parts[7..],
        [
            "now",
            "0",
            "function now() { [native code] }",
            "Date",
            "7",
            "function Date() { [native code] }"
        ]
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

/// A page's realm holds what a real app does (2026-11-21): 300 MiB, past
/// the 256 MiB a realm had, within its 1 GiB.
#[test]
fn a_realm_holds_more_than_a_quarter_gibibyte() {
    let mut realm = realm();
    let mut host = Recorder::default();
    realm
        .run(
            &script("globalThis.big = new Uint8Array(300 << 20); big[big.length - 1] = 1;"),
            &mut host,
        )
        .expect("it runs");
    assert!(realm.heap() > 300 << 20);
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

/// A host that answers nothing of a document: a fuzz target's, or a
/// realm's before it has one.
#[derive(Debug, Default)]
struct Bare {
    console: Vec<String>,
}

impl ScriptingHost for Bare {
    fn console(&mut self, _: ConsoleLevel, message: String) {
        self.console.push(message);
    }

    fn title(&mut self) -> String {
        String::new()
    }

    fn set_title(&mut self, _: String) {}
}

#[test]
fn a_host_without_a_document_lets_code_from_strings_compile() {
    let mut realm: Box<dyn Realm<Bare>> = QuickJs.realm(Limits::default()).expect("a realm");
    let mut host = Bare::default();
    let source = "console.log(eval('1 + 1'), new Function('return 3')(), \
                  typeof new WebAssembly.Module(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0])));";
    realm.run(&script(source), &mut host).expect("it runs");
    assert_eq!(host.console, ["2 3 object"]);
}

#[test]
fn an_errors_stack_takes_any_value_as_the_reference_browsers_does() {
    // QuickJS-ng's setter on Error.prototype threw for a value not a
    // string (twitch's player): Chrome 156 keeps whatever is set.
    let done = realm()
        .run(
            &script(
                "var own = function (o) { return Object.prototype.hasOwnProperty.call(o, 'stack'); };\
                 var e = new Error('x'); e.stack = undefined;\
                 var f = new Error('x'); f.stack = { a: 1 };\
                 var o = Object.create(Error.prototype); var before = String(o.stack); o.stack = 5;\
                 class E extends Error {} var g = new E('y'); g.stack = null;\
                 var s = new Error('z'); s.stack = 'mine';\
                 Error.prototype.stack = 7; var p = Error.prototype.stack;\
                 [String(e.stack), own(e), typeof f.stack, before, o.stack, own(o), String(g.stack),\
                  s.stack, p].join()",
            ),
            &mut Recorder::default(),
        )
        .expect("it runs");
    assert_eq!(
        done,
        Completion::String("undefined,true,object,undefined,5,true,null,mine,7".to_owned())
    );
}

/// The scripts' sampler (the `js-sampler` tool): on, a hot loop in `a.js`
/// samples as `a.js`; a page that overrides what an error's stack reads
/// sees nothing of it.
#[cfg(feature = "js-sampler")]
mod sampler_tests {
    use super::*;
    use crate::quickjs::sampler_for_tests as sampler;

    fn sampled(source: &str) -> (Completion, Vec<(String, u64)>) {
        sampler::force(Some(1));
        let mut realm = realm();
        sampler::force(None);
        realm
            .run(
                &script(
                    "function warm() { var x = 0; for (var i = 0; i < 1e5; i++) x += i; return x; }\
                     warm();",
                ),
                &mut Recorder::default(),
            )
            .expect("the page's own runs");
        let hot = Script {
            url: "https://example.com/a.js",
            ..script(source)
        };
        let done = realm.run(&hot, &mut Recorder::default()).expect("it runs");
        (done, realm.samples())
    }

    const HOT_FN: &str = "function hot() { var x = 0; for (var i = 0; i < 3e6; i++) x = (x + i) % 1000003; return x; }";
    const HOT: &str = "function hot() { var x = 0; for (var i = 0; i < 3e6; i++) x = (x + i) % 1000003; return x; }\
                       String(hot());";

    #[test]
    fn a_hot_loop_samples_as_its_script() {
        let (_, samples) = sampled(HOT);
        let total: u64 = samples.iter().map(|(_, n)| n).sum();
        let a: u64 = samples
            .iter()
            .filter(|(url, _)| url.ends_with("/a.js"))
            .map(|(_, n)| n)
            .sum();
        assert!(total >= 10, "{samples:?}");
        assert!(a * 10 >= total * 9, "{samples:?}");
    }

    #[test]
    fn a_page_overriding_the_stack_sees_nothing_of_the_sampler() {
        let watch = "var calls = 0;\
                     Error.prepareStackTrace = function () { calls++; return ''; };\
                     Error.stackTraceLimit = 50;\
                     Object.defineProperty(Error.prototype, 'stack', { get: function () { calls++; }, configurable: true });\
                     var RealError = Error; Error = function () { calls++; return new RealError(); };";
        let (done, samples) = sampled(&format!("{watch}{HOT_FN} String(hot()) + ' ' + calls"));
        assert!(!samples.is_empty());
        let (plain, _) = sampled(HOT);
        let Completion::String(plain) = plain else {
            panic!("{plain:?}")
        };
        assert_eq!(done, Completion::String(format!("{plain} 0")));
    }
}

/// A realm whose bootstrap its heap refuses is not made, and goes
/// cleanly: a debug build's guarded natives go before its context, or
/// QuickJS finds the realm's objects still held as its runtime goes.
#[test]
fn a_realm_its_heap_refuses_goes_cleanly() {
    for heap in [64 << 10, 1 << 20] {
        let made = QuickJs.realm::<Recorder>(Limits {
            heap,
            ..Limits::default()
        });
        assert!(made.is_err(), "a heap of {heap} bytes");
    }
}

/// G90 phase 1: two realms of one agent share its runtime: one job
/// queue, one heap and one budget; each keeps its own global.
mod agent_tests {
    use super::*;

    fn two(limits: Limits) -> (Box<dyn Realm<Recorder>>, Box<dyn Realm<Recorder>>) {
        let agent = QuickJs.agent::<Recorder>(limits, None).expect("an agent");
        let a = QuickJs
            .realm_in(&agent, crate::RealmKind::Window, true)
            .expect("a realm");
        let b = QuickJs
            .realm_in(&agent, crate::RealmKind::Window, true)
            .expect("another");
        (a, b)
    }

    fn number(realm: &mut Box<dyn Realm<Recorder>>, source: &str) -> Completion {
        realm
            .run(&script(source), &mut Recorder::default())
            .expect("it runs")
    }

    #[test]
    fn realms_of_an_agent_keep_their_own_globals() {
        let (mut a, mut b) = two(Limits::default());
        number(&mut a, "var mine = 'a'");
        assert_eq!(
            number(&mut b, "typeof mine"),
            Completion::String("undefined".into())
        );
        assert_eq!(number(&mut a, "mine"), Completion::String("a".into()));
    }

    /// A promise of one realm's settles at the other's checkpoint: the
    /// job queue is the agent's.
    #[test]
    fn realms_of_an_agent_share_its_job_queue() {
        let (mut a, mut b) = two(Limits::default());
        number(
            &mut a,
            "var done = 'no'; Promise.resolve().then(() => { done = 'yes' }); 0",
        );
        assert!(b.checkpoint(&mut Recorder::default()).is_empty());
        assert_eq!(number(&mut a, "done"), Completion::String("yes".into()));
    }

    /// What one realm holds counts against the other's room: the heap
    /// is the agent's, and grows by [`crate::REALM_HEAP`] for its second
    /// realm, so that one of two holds what a realm of its own could not.
    #[test]
    fn realms_of_an_agent_share_its_heap() {
        let limits = Limits {
            heap: 64 << 20,
            ..Limits::default()
        };
        let (mut a, mut b) = two(limits);
        let hold =
            "var kept = []; for (var i = 0; i < 70; i++) kept.push('x'.repeat(1 << 20) + i); 1";
        number(&mut a, hold);
        let error = b
            .run(&script(hold), &mut Recorder::default())
            .expect_err("no room left");
        assert_eq!(error.kind, ErrorKind::Heap, "{error:?}");
        // A realm of its own has 64 MiB, not the 70 held above.
        let mut alone = realm_with(limits);
        let error = alone
            .run(&script(hold), &mut Recorder::default())
            .expect_err("no room");
        assert_eq!(error.kind, ErrorKind::Heap, "{error:?}");
    }

    /// An error thrown in a realm of an agent of two, holding more than a
    /// realm of its own may, is an error, not the agent's death: its heap
    /// is nearly full of the agent's limit only, which grew with it.
    #[test]
    fn an_error_in_a_grown_agent_is_not_its_death() {
        let limits = Limits {
            heap: 64 << 20,
            ..Limits::default()
        };
        let (mut a, mut b) = two(limits);
        number(
            &mut a,
            "var kept = []; for (var i = 0; i < 60; i++) kept.push('x'.repeat(1 << 20) + i); 1",
        );
        let error = a
            .run(
                &script("throw new Error('only this')"),
                &mut Recorder::default(),
            )
            .expect_err("thrown");
        assert_ne!(error.kind, ErrorKind::Heap, "{error:?}");
        assert!(a.dead().is_none());
        assert_eq!(
            number(&mut b, "String(1 + 1)"),
            Completion::String("2".into())
        );
    }

    /// The heap an agent's realms share follows how many there are: the
    /// first's limit, 64 MiB more for each after it, up to 1 GiB; as they
    /// go, back down, but never under what the runtime holds.
    #[test]
    fn an_agents_heap_grows_and_shrinks_with_its_realms() {
        let limits = Limits {
            heap: 64 << 20,
            ..Limits::default()
        };
        let agent = QuickJs.agent::<Recorder>(limits, None).expect("an agent");
        let made = |count: usize| -> Vec<Box<dyn Realm<Recorder>>> {
            (0..count)
                .map(|_| {
                    QuickJs
                        .realm_in(&agent, crate::RealmKind::Window, true)
                        .expect("a realm")
                })
                .collect()
        };
        let three = made(3);
        assert_eq!(agent.realm_count(), 3);
        assert_eq!(agent.heap_limit(), (64 << 20) + 2 * crate::REALM_HEAP);
        drop(three);
        assert_eq!(agent.realm_count(), 0);
        let one = made(1);
        assert_eq!(agent.heap_limit(), 64 << 20);
        drop(one);
        // Many: capped.
        let many = made(40);
        assert_eq!(agent.heap_limit(), crate::MAX_AGENT_HEAP);
        drop(many);
    }

    /// A realm's objects collected in another realm's run (G90): their
    /// finalizers run in the agent's one job queue, while the document
    /// of the realm they are of may not be lent. The nodes they let go
    /// of are released to it when it is next, nothing thrown and no call
    /// without its host counted (Google Images, 2026-11-19: "no script
    /// is running" from the bootstrap's `released`, twice a load).
    #[test]
    fn a_realms_objects_collected_in_anothers_run_are_released_to_it_later() {
        let (mut a, mut b) = two(Limits::default());
        let mut a_host = Recorder::default();
        let mut b_host = Recorder::default();
        let _ = crate::take_unlent_calls();
        b.run(
            &script("for (let i = 0; i < 10; i++) document.createElement('p'); 0"),
            &mut b_host,
        )
        .expect("it runs");
        let made: Vec<u64> = b_host
            .tree
            .exposed
            .iter()
            .copied()
            .filter(|&h| h != 0)
            .collect();
        assert_eq!(made.len(), 10, "{:?}", b_host.tree.exposed);
        assert!(
            b_host.tree.released.is_empty(),
            "{:?}",
            b_host.tree.released
        );
        // The other realm's sweep collects them, and runs their finalizers.
        a.sweep(&mut a_host, true);
        assert!(a_host.reported.is_empty(), "{:?}", a_host.reported);
        assert_eq!(crate::take_unlent_calls(), 0);
        assert!(b_host.tree.released.is_empty());
        // The realm's next run hands them over first.
        b.run(&script("0"), &mut b_host).expect("it runs");
        let mut released = b_host.tree.released.clone();
        released.sort_unstable();
        let mut made = made;
        made.sort_unstable();
        assert_eq!(released, made);
        assert!(b_host.reported.is_empty(), "{:?}", b_host.reported);
        // And when its document is next lent for another realm's run.
        b.run(&script("document.createElement('i'); 0"), &mut b_host)
            .expect("it runs");
        let before = b_host.tree.released.len();
        a.sweep(&mut a_host, true);
        assert_eq!(crate::take_unlent_calls(), 0);
        b.lend(std::mem::take(&mut b_host));
        let b_host = b.reclaim().expect("lent");
        assert_eq!(b_host.tree.released.len(), before + 1);
    }

    /// The budget stops a run in either realm, and is the next run's
    /// again whichever realm it is in.
    #[test]
    fn realms_of_an_agent_share_its_budget() {
        let limits = Limits {
            steps: 50,
            ..Limits::default()
        };
        let (mut a, mut b) = two(limits);
        let error = a
            .run(&script("for (;;) {}"), &mut Recorder::default())
            .expect_err("stopped");
        assert_eq!(error.kind, ErrorKind::Budget, "{error:?}");
        assert_eq!(
            number(&mut b, "String(1 + 1)"),
            Completion::String("2".into())
        );
        let error = b
            .run(&script("for (;;) {}"), &mut Recorder::default())
            .expect_err("stopped");
        assert_eq!(error.kind, ErrorKind::Budget, "{error:?}");
    }
}

/// G90 phase 2: a realm reaches a frame of its agent as itself, its
/// document through the host the frame's realm is lent.
mod frame_tests {
    use super::*;

    #[test]
    fn a_frame_of_the_agent_is_reached_as_itself() {
        let agent = QuickJs
            .agent::<Recorder>(Limits::default(), None)
            .expect("an agent");
        let mut top = QuickJs
            .realm_in(&agent, crate::RealmKind::Window, true)
            .expect("a realm");
        let mut child = QuickJs
            .realm_in(&agent, crate::RealmKind::Window, true)
            .expect("another");
        top.name_frame(Some(0));
        child.name_frame(Some(5));
        let mut child_host = Recorder {
            frames: Some((5, Vec::new())),
            title: "bee".into(),
            ..Recorder::default()
        };
        child
            .run(&script("var made = 'in the frame'"), &mut child_host)
            .expect("it runs");
        child.lend(child_host);
        let mut host = Recorder {
            frames: Some((0, vec![5])),
            ..Recorder::default()
        };
        top.enter(&Task::FrameList(vec![String::new()]), &mut host)
            .expect("the window's frames");
        let done = top
            .run(
                &script(
                    "var w = frames[0]; \
                     [typeof w.TextEncoder, w.TextEncoder === TextEncoder, w.made, \
                      w.document.title, w.Array === Array].join()",
                ),
                &mut host,
            )
            .expect("it runs");
        assert_eq!(
            done,
            Completion::String("function,false,in the frame,bee,false".into())
        );
        assert!(child.reclaim().is_some());
    }

    /// G90 phase 3 (b): a document of the agent with no realm, lent to
    /// the agent, has its realm made as a run first reaches it, the
    /// document lent to it; the frame's engine takes the realm after, and
    /// the next reach is the same window.
    #[test]
    fn a_frame_with_no_realm_has_one_made_as_it_is_reached() {
        let agent = QuickJs
            .agent::<Recorder>(Limits::default(), None)
            .expect("an agent");
        let mut top = QuickJs
            .realm_in(&agent, crate::RealmKind::Window, true)
            .expect("a realm");
        top.name_frame(Some(0));
        let child_host = Recorder {
            frames: Some((5, Vec::new())),
            title: "bee".into(),
            ..Recorder::default()
        };
        agent.lend_unborn(5, child_host);
        let mut host = Recorder {
            frames: Some((0, vec![5])),
            ..Recorder::default()
        };
        top.enter(&Task::FrameList(vec![String::new()]), &mut host)
            .expect("the window's frames");
        assert_eq!(agent.realm_count(), 1);
        let done = top
            .run(
                &script(
                    "var w = frames[0]; \
                     [typeof w.TextEncoder, w.TextEncoder === TextEncoder, \
                      String(w.TextEncoder).includes('[native code]'), \
                      w.document.title, w.Array === Array, frames[0] === w].join()",
                ),
                &mut host,
            )
            .expect("it runs");
        assert_eq!(
            done,
            Completion::String("function,false,true,bee,false,true".into())
        );
        assert_eq!(agent.realm_count(), 2);
        // The heap grew with it in the reaching run, under its lock.
        let grown = Limits::default().heap + crate::REALM_HEAP;
        assert_eq!(agent.heap_limit(), grown);
        assert_eq!(agent.runtime_limit(), i64::try_from(grown).unwrap());
        assert!(agent.take_unborn(5).is_none(), "lent to its realm");
        let mut child = agent.take_made(5).expect("made");
        let child_host = child.reclaim().expect("its document, lent to it");
        assert_eq!(child_host.title, "bee");
        // The frame's engine runs it as its own from now on.
        let mut child_host = child_host;
        assert_eq!(
            child
                .run(&script("String(typeof TextEncoder)"), &mut child_host)
                .expect("it runs"),
            Completion::String("function".into())
        );
        // A frame of no document lent is another agent's: no realm made.
        let mut host = Recorder {
            frames: Some((0, vec![9])),
            ..Recorder::default()
        };
        top.enter(&Task::FrameList(vec![String::new()]), &mut host)
            .expect("the window's frames");
        let done = top
            .run(
                &script("try { frames[0].document; 'read' } catch (e) { e.name }"),
                &mut host,
            )
            .expect("it runs");
        // (This host answers none of the cross-origin window's questions.)
        assert_ne!(done, Completion::String("read".into()));
        assert_eq!(agent.realm_count(), 2);
        assert!(!agent.has_made(9));
    }

    /// Realms made by reach and dropped, their frames gone, then the agent:
    /// the runtime goes with nothing of theirs held (QuickJS asserts as it
    /// frees a runtime that still holds an object, in a debug build).
    #[test]
    fn realms_made_by_reach_go_with_their_frames_and_their_agent() {
        let agent = QuickJs
            .agent::<Recorder>(Limits::default(), None)
            .expect("an agent");
        let mut top = QuickJs
            .realm_in(&agent, crate::RealmKind::Window, true)
            .expect("a realm");
        top.name_frame(Some(0));
        let frames: Vec<u32> = (1..=4).collect();
        for &frame in &frames {
            agent.lend_unborn(
                frame,
                Recorder {
                    frames: Some((frame, Vec::new())),
                    ..Recorder::default()
                },
            );
        }
        let mut host = Recorder {
            frames: Some((0, frames.clone())),
            ..Recorder::default()
        };
        top.enter(&Task::FrameList(vec![String::new(); 4]), &mut host)
            .expect("the window's frames");
        top.run(
            &script(
                "var kept = [0, 1, 2, 3].map(i => frames[i]); \
                 kept.forEach((w, i) => { w.mine = { i, back: window }; }); 0",
            ),
            &mut host,
        )
        .expect("it runs");
        assert_eq!(agent.realm_count(), 5);
        // Two frames' engines take theirs, then drop them (removed); the
        // others' realms go unclaimed.
        for frame in [1, 2] {
            let mut realm = agent.take_made(frame).expect("made");
            assert!(realm.reclaim().is_some());
            drop(realm);
        }
        let (dropped, unborn) = agent.drop_unclaimed();
        assert_eq!((dropped, unborn.len()), (2, 0));
        assert_eq!(agent.realm_count(), 1);
        drop(top);
        drop(agent);
    }
}

/// A realm a thread-local holds goes as its thread ends, perhaps after
/// the realms' own thread-locals went: its drop reaches none of them
/// destroyed (the clone fuzz target's, CI 2026-11-20: "cannot access a
/// Thread Local Storage value during or after destruction").
#[test]
fn a_realm_a_thread_local_holds_goes_with_its_thread() {
    thread_local! {
        static HELD: std::cell::RefCell<Option<Box<dyn Realm<Recorder>>>> =
            const { std::cell::RefCell::new(None) };
    }
    let ended = std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(|| {
            // Held first, so that its destructor is registered before
            // the realm's thread-locals', and may run after theirs.
            HELD.with(|held| held.borrow().is_none());
            let realm = QuickJs
                .realm_of::<Recorder>(crate::RealmKind::Window, Limits::default(), None)
                .expect("a realm");
            HELD.with(|held| *held.borrow_mut() = Some(realm));
        })
        .expect("a thread")
        .join();
    assert!(ended.is_ok(), "the thread's end panicked");
}

/// Errors' stacks and the commonest TypeErrors' messages are the
/// reference browser's (2026-11-20, probed in Chrome 156): the first
/// line the error's name and message, a frame `at f (url:line:column)`,
/// a script's own `at url:line:column`, code from a string `at eval
/// (eval at f (url:line:column), <anonymous>:line:column)`; and
/// `Error.prepareStackTrace` undefined until a page sets one, which is
/// called instead. The columns are QuickJS's: a `new` expression's at
/// its callee, where the reference's is at `new`.
#[test]
fn stacks_and_messages_are_written_as_the_reference_writes_them() {
    let mut realm = realm();
    let mut host = Recorder::default();
    let run = |realm: &mut Box<dyn Realm<Recorder>>, host: &mut Recorder, source: &str| match realm
        .run(&script(source), host)
        .expect("it runs")
    {
        Completion::String(text) => text,
        other => panic!("not a string: {other:?}"),
    };
    // Error's own properties as V8's, before a page sets any.
    assert_eq!(
        run(
            &mut realm,
            &mut host,
            "['prepareStackTrace' in Error, Object.getOwnPropertyNames(Error).join(),\
             JSON.stringify(Object.getOwnPropertyDescriptor(Error, 'stackTraceLimit'))].join('|')"
        ),
        "false|length,name,prototype,captureStackTrace,isError,stackTraceLimit|\
         {\"value\":10,\"writable\":true,\"enumerable\":true,\"configurable\":true}"
    );
    assert_eq!(
        run(
            &mut realm,
            &mut host,
            "function named() {\n  return new Error('x').stack;\n}\nnamed()"
        ),
        "Error: x\n    at named (https://example.com/page.html:2:14)\n    \
         at https://example.com/page.html:4:1"
    );
    let from_string = run(
        &mut realm,
        &mut host,
        "new Function('return new Error(\"f\").stack')()",
    );
    assert!(
        from_string.starts_with(
            "Error: f\n    at eval (eval at <anonymous> (https://example.com/page.html:1:"
        ) && from_string.contains("), <anonymous>:3:"),
        "{from_string}"
    );
    assert_eq!(
        run(
            &mut realm,
            &mut host,
            "[() => null.length, () => undefined.x, () => { null.y = 1; }, () => '' + Symbol()]\
             .map(f => { try { f(); } catch (e) { return e.message; } }).join('|')"
        ),
        "Cannot read properties of null (reading 'length')|Cannot read properties of undefined \
         (reading 'x')|Cannot set properties of null (setting 'y')|Cannot convert a Symbol value \
         to a string"
    );
    assert_eq!(
        run(
            &mut realm,
            &mut host,
            "const before = typeof Error.prepareStackTrace;\
             Error.prepareStackTrace = (e, frames) => 'mine ' + frames.length;\
             const mine = new Error('p').stack;\
             Error.prepareStackTrace = undefined;\
             Error.stackTraceLimit = 0;\
             const none = new Error('l').stack;\
             Error.stackTraceLimit = 10;\
             [before, mine.slice(0, 5), none, String(Error.prepareStackTrace)].join('|')"
        ),
        "undefined|mine |Error: l|undefined"
    );
}

/// A stack is written whatever a page put on the built-ins' prototypes
/// (WPT adoptedstylesheets-observablearray sets an accessor on
/// Array.prototype's indices and `get` on Object.prototype), and is a
/// string for an object QuickJS gives an own stack (a DOMException), as
/// the WebGPU CTS checks.
#[test]
fn a_stack_is_written_whatever_the_page_put_on_the_built_ins() {
    let mut realm = realm();
    let mut host = Recorder::default();
    let answer = realm
        .run(
            &script(
                "Object.defineProperty(Array.prototype, '0', { get() { return 'page'; },\
                   set() { throw new Error('set'); }, configurable: true });\
                 Object.defineProperty(Object.prototype, 'get', { value: 1, configurable: true });\
                 Error.prepareStackTrace = (e, frames) => typeof frames[0].getFileName;\
                 const custom = new Error('c').stack;\
                 Error.prepareStackTrace = undefined;\
                 const plain = new Error('p').stack;\
                 delete Array.prototype[0];\
                 delete Object.prototype.get;\
                 [custom, plain.slice(0, 8), typeof new DOMException('m', 'NotFoundError').stack]\
                   .join('|')",
            ),
            &mut host,
        )
        .expect("it runs");
    assert_eq!(
        answer,
        Completion::String("function|Error: p|string".to_owned())
    );
}

/// The null trace (a sweep build's, `SILVER_TRACE_NULLS`) is off unless
/// its variable is set: a null read warns nothing.
#[test]
fn the_null_trace_is_off_without_its_variable() {
    if std::env::var_os("SILVER_TRACE_NULLS").is_some() {
        return;
    }
    let mut realm = realm();
    let mut host = Recorder::default();
    realm
        .run(
            &script("try { null.x; } catch (e) {} try { undefined.y; } catch (e) {}"),
            &mut host,
        )
        .expect("it runs");
    let _ = realm.checkpoint(&mut host);
    assert!(
        host.console
            .iter()
            .all(|(_, line)| !line.contains("NULLTRACE")),
        "{:?}",
        host.console
    );
}
