//! Web Storage in the engine (M28.2): `localStorage` and
//! `sessionStorage` as the reference browser answered the same calls
//! (scratchpad m28 `p1-storage.html`, Chrome 154), where they differ
//! as Web IDL says; the quota; opaque origins refused; the areas kept
//! and not kept across engines; and the `storage` event.

use std::time::Duration;

use net::Url;
use net::test_server::{Reply, TestServer};
use profile::{Memory, Profile};

use crate::LoadState;
use crate::testing::*;

/// Loads `url` in `tab` until it is shown and loaded.
fn load(tab: &mut Tab, url: Url) {
    tab.engine.navigate(url.clone());
    tab.wait_until(|engine| {
        incoming(engine).is_none()
            && engine.url() == Some(&url)
            && engine.load_state() == &LoadState::Loaded
    });
}

/// The console's lines, taken.
fn console(tab: &mut Tab) -> Vec<String> {
    tab.engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

/// A tab with scripts on, keeping what it keeps in `memory` if given.
fn scripted(memory: Option<&Memory>) -> Tab {
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    if let Some(memory) = memory {
        tab.engine
            .set_profile(Profile::in_memory(memory).unwrap())
            .unwrap();
    }
    tab
}

/// A server whose `/page.html` runs `script` after a helper that logs
/// each step as `name=` and its value in JSON, or what it threw.
fn page(script: &'static str) -> TestServer {
    serve(move |path| match path {
        "/page.html" => Some(html(&format!(
            "<script>const ls = localStorage, ss = sessionStorage;\n\
             const step = (name, f) => {{ let v; try {{ v = JSON.stringify(f()); }} \
             catch (e) {{ v = 'threw ' + e.name + ': ' + e.message; }} \
             console.log(name + '=' + v); }};\n{script}</script>"
        ))),
        _ => None,
    })
}

/// The lines `script` logs on `tab`'s page.
fn run(tab: &mut Tab, script: &'static str) -> Vec<String> {
    let server = page(script);
    load(tab, server.url("/page.html"));
    console(tab)
}

#[test]
fn storage_answers_as_the_reference_browser_and_web_idl_say() {
    let mut tab = scripted(None);
    let lines = run(
        &mut tab,
        r#"
step("sameObject", () => ls === window.localStorage && ss === window.sessionStorage && ls !== ss);
step("toString", () => Object.prototype.toString.call(ls) + " " + String(ls));
step("protoNames", () => Object.getOwnPropertyNames(Storage.prototype).sort().join());
ls.setItem("b", 1); ls.setItem("a", {}); ls.c = "named"; ls.setItem("", "empty");
step("order", () => [0, 1, 2, 3, 4].map(i => ls.key(i)));
step("length", () => ls.length);
step("get", () => [ls.getItem("a"), ls.getItem("nope"), ls.b, typeof ls.nope, ls[""]]);
step("in", () => ["a" in ls, "nope" in ls, "getItem" in ls, "length" in ls]);
step("keys", () => Object.keys(ls));
step("forIn", () => { const k = []; for (const x in ls) k.push(x); return k; });
step("desc", () => Object.getOwnPropertyDescriptor(ls, "b"));
step("shadowGet", () => { ls.setItem("getItem", "x"); return [typeof ls.getItem, ls.getItem("getItem")]; });
step("shadowSet", () => { ls.key = "y"; return [typeof ls.key, ls.getItem("key")]; });
step("lengthItem", () => { ls.setItem("length", "9"); return [ls.length, ls.getItem("length")]; });
step("keysHidden", () => [Object.keys(ls), Object.getOwnPropertyNames(ls)]);
step("deleteNamed", () => [delete ls.b, ls.getItem("b"), delete ls.nope]);
step("deleteBuiltin", () => [delete ls.getItem, typeof ls.getItem, ls.getItem("getItem")]);
step("keyRange", () => [ls.key(99), ls.key(-1), ls.key(4294967297), ls.key("1")]);
step("numberIndex", () => { ls.setItem("0", "zero"); return [ls[0], ls.key(0)]; });
step("defineProperty", () => { Object.defineProperty(ls, "dp", { value: "v" }); return ls.getItem("dp"); });
step("defineAccessor", () => { Object.defineProperty(ls, "acc", { get() { return 1; } }); return ls.getItem("acc"); });
step("symbol", () => { const s = Symbol("s"); ls[s] = "sym"; return [ls[s], ls.length, Object.getOwnPropertySymbols(ls).length]; });
step("setReturn", () => { ls.n = 5; return [typeof ls.n, ls.n]; });
step("nullValue", () => { ls.setItem("nl", null); ls.setItem("ud", undefined); return [ls.getItem("nl"), ls.getItem("ud")]; });
step("missingArgs", () => ls.setItem("x"));
step("getItemArgs", () => ls.getItem());
step("preventExtensions", () => Object.preventExtensions(ls));
step("newStorage", () => new Storage());
step("illegal", () => Storage.prototype.getItem.call({}, "a"));
step("inherited", () => { const child = Object.create(ls); child.own = "1"; return [ls.getItem("own"), child.own]; });
step("separate", () => { ss.setItem("s", "1"); return [ls.getItem("s"), ss.length, ss.getItem("s")]; });
step("sameDocEvent", () => { let n = 0; addEventListener("storage", () => n++); ls.setItem("ev", "1"); return n; });
ls.clear(); ss.clear();
step("cleared", () => [ls.length, ss.length, ls.key(0)]);
"#,
    );
    let expected = [
        r#"sameObject=true"#,
        r#"toString="[object Storage] [object Storage]""#,
        r#"protoNames="clear,constructor,getItem,key,length,removeItem,setItem""#,
        // Insertion order, where the reference browser's is its hash's.
        r#"order=["b","a","c","",null]"#,
        r#"length=4"#,
        r#"get=["[object Object]",null,"1","undefined","empty"]"#,
        r#"in=[true,false,true,true]"#,
        r#"keys=["b","a","c",""]"#,
        r#"forIn=["b","a","c","","length","clear","getItem","key","removeItem","setItem"]"#,
        r#"desc={"value":"1","writable":true,"enumerable":true,"configurable":true}"#,
        r#"shadowGet=["function","x"]"#,
        // Web IDL's [[Set]] stores the item; the reference browser makes
        // an own property `key`.
        r#"shadowSet=["function","y"]"#,
        r#"lengthItem=[7,"9"]"#,
        // Items a prototype's members hide are not own keys either.
        r#"keysHidden=[["b","a","c",""],["b","a","c",""]]"#,
        r#"deleteNamed=[true,null,true]"#,
        r#"deleteBuiltin=[true,"function","x"]"#,
        r#"keyRange=[null,null,"c","c"]"#,
        r#"numberIndex=["zero","a"]"#,
        r#"defineProperty="v""#,
        "defineAccessor=threw TypeError: Failed to set a named property 'acc' on 'Storage': \
         Accessor properties are not allowed.",
        r#"symbol=["sym",8,1]"#,
        r#"setReturn=["string","5"]"#,
        r#"nullValue=["null","undefined"]"#,
        "missingArgs=threw TypeError: Failed to execute 'setItem' on 'Storage': \
         2 arguments required, but only 1 present.",
        "getItemArgs=threw TypeError: Failed to execute 'getItem' on 'Storage': \
         1 argument required, but only 0 present.",
        "preventExtensions=threw TypeError: Cannot prevent extensions",
        "newStorage=threw TypeError: Failed to construct 'Storage': Illegal constructor",
        "illegal=threw TypeError: Illegal invocation",
        r#"inherited=[null,"1"]"#,
        r#"separate=[null,1,"1"]"#,
        r#"sameDocEvent=0"#,
        r#"cleared=[0,0,null]"#,
    ];
    assert_eq!(lines, expected);
}

#[test]
fn an_area_holds_five_mebi_code_units_and_says_so_past_them() {
    let mut tab = scripted(None);
    let lines = run(
        &mut tab,
        r#"
step("fits", () => { ls.setItem("q", "x".repeat(5242879)); return ls.getItem("q").length; });
step("over", () => { try { ls.setItem("q", "x".repeat(5242880)); return "fit"; } catch (e) {
  return [e.name, e.constructor.name, e.code, e.message, e instanceof DOMException,
    e instanceof QuotaExceededError, e.quota, e.requested, ls.getItem("q").length]; } });
step("named", () => { ls.q = "x".repeat(5242880); });
step("keyCounts", () => { ls.clear(); ls.setItem("qqqqqqqqqq", "x".repeat(5242870)); return ls.length; });
step("keyOver", () => ls.setItem("qqqqqqqqqq", "x".repeat(5242871)));
step("sessionApart", () => { ss.setItem("q", "x".repeat(5242870)); return ss.getItem("q").length; });
step("bmp", () => { ls.clear(); ls.setItem("q", "\u00e9".repeat(5242879)); return ls.length; });
step("astral", () => { ls.clear(); ls.setItem("q", "\u{1F600}".repeat(2621439)); return ls.length; });
step("astralOver", () => { ls.clear(); ls.setItem("q", "\u{1F600}".repeat(2621440)); });
step("replaced", () => { ls.clear(); ls.setItem("a", "x".repeat(4000000)); ls.setItem("a", "y".repeat(5242879)); return ls.getItem("a")[0]; });
step("afterRemove", () => { ls.removeItem("a"); ls.setItem("b", "z".repeat(5242879)); return ls.length; });
ls.clear(); ss.clear();
step("ctor", () => { const e = new QuotaExceededError("m", { quota: 1, requested: 2 });
  return [e.name, e.message, e.code, e.quota, e.requested, Object.prototype.toString.call(e)]; });
step("ctorDefaults", () => { const e = new QuotaExceededError(); return [e.message, e.quota, e.requested]; });
step("negative", () => new QuotaExceededError("m", { quota: -1 }));
step("less", () => new QuotaExceededError("m", { quota: 2, requested: 1 }));
"#,
    );
    let over =
        "Failed to execute 'setItem' on 'Storage': Setting the value of 'q' exceeded the quota.";
    let expected = [
        "fits=5242879".to_owned(),
        format!(
            r#"over=["QuotaExceededError","QuotaExceededError",22,"{over}",true,true,null,null,5242879]"#
        ),
        "named=threw QuotaExceededError: Failed to set a named property 'q' on 'Storage': \
         Setting the value of 'q' exceeded the quota."
            .to_owned(),
        "keyCounts=1".to_owned(),
        "keyOver=threw QuotaExceededError: Failed to execute 'setItem' on 'Storage': \
         Setting the value of 'qqqqqqqqqq' exceeded the quota."
            .to_owned(),
        "sessionApart=5242870".to_owned(),
        "bmp=1".to_owned(),
        "astral=1".to_owned(),
        format!("astralOver=threw QuotaExceededError: {over}"),
        r#"replaced="y""#.to_owned(),
        "afterRemove=1".to_owned(),
        r#"ctor=["QuotaExceededError","m",22,1,2,"[object QuotaExceededError]"]"#.to_owned(),
        r#"ctorDefaults=["",null,null]"#.to_owned(),
        "negative=threw RangeError: Failed to construct 'QuotaExceededError': \
         The quota must not be negative."
            .to_owned(),
        "less=threw RangeError: Failed to construct 'QuotaExceededError': \
         The requested amount must not be less than the quota."
            .to_owned(),
    ];
    assert_eq!(lines, expected);
}

#[test]
fn the_storage_event_interface_is_web_idls() {
    let mut tab = scripted(None);
    let lines = run(
        &mut tab,
        r#"
step("ctor", () => { const e = new StorageEvent("storage", { key: "k", oldValue: "o", newValue: "n",
  url: "u", storageArea: ls }); return [e.key, e.oldValue, e.newValue, e.url, e.storageArea === ls, e.bubbles]; });
step("defaults", () => { const e = new StorageEvent("x"); return [e.key, e.oldValue, e.newValue, e.url, e.storageArea]; });
step("badArea", () => new StorageEvent("x", { storageArea: {} }));
step("init", () => { const e = new StorageEvent("x"); e.initStorageEvent("y", true, true, "k", null, "n", "u", ss);
  return [e.type, e.bubbles, e.cancelable, e.key, e.oldValue, e.newValue, e.url, e.storageArea === ss]; });
step("initBadArea", () => new StorageEvent("x").initStorageEvent("y", false, false, null, null, null, "", {}));
step("created", () => { const e = document.createEvent("StorageEvent"); return [e instanceof StorageEvent, e.key, e.url]; });
step("handler", () => "onstorage" in window && window.onstorage === null);
"#,
    );
    assert_eq!(
        lines,
        [
            r#"ctor=["k","o","n","u",true,false]"#,
            r#"defaults=[null,null,null,"",null]"#,
            "badArea=threw TypeError: Failed to construct 'StorageEvent': Failed to read the \
             'storageArea' property from 'StorageEventInit': Failed to convert value to 'Storage'.",
            r#"init=["y",true,true,"k",null,"n","u",true]"#,
            "initBadArea=threw TypeError: Failed to execute 'initStorageEvent' on 'StorageEvent': \
             parameter 8 is not of type 'Storage'.",
            r#"created=[true,null,""]"#,
            "handler=true",
        ]
    );
}

#[test]
fn an_opaque_origin_has_no_storage() {
    let mut tab = scripted(None);
    let script = "<script>for (const name of ['localStorage', 'sessionStorage']) { \
                  try { window[name]; console.log(name + ' read'); } \
                  catch (e) { console.log(e.name + ': ' + e.message); } }</script>";
    load(
        &mut tab,
        Url::parse(&format!("data:text/html,{script}")).unwrap(),
    );
    let data = |name: &str| {
        format!(
            "SecurityError: Failed to read the '{name}' property from 'Window': \
             Storage is disabled inside 'data:' URLs."
        )
    };
    assert_eq!(
        console(&mut tab),
        [data("localStorage"), data("sessionStorage")]
    );
    // A file: page's origin is opaque too: its storage would be every
    // local file's.
    let dir = std::env::temp_dir().join(format!("silver-storage-file-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("page.html");
    std::fs::write(&file, script).unwrap();
    load(&mut tab, Url::from_file_path(&file).unwrap());
    let denied = |name: &str| {
        format!(
            "SecurityError: Failed to read the '{name}' property from 'Window': \
             Access is denied for this document."
        )
    };
    assert_eq!(
        console(&mut tab),
        [denied("localStorage"), denied("sessionStorage")]
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn local_storage_outlives_the_engine_and_session_storage_does_not() {
    let server = serve(|path| match path {
        "/set.html" => Some(html(
            "<script>localStorage.setItem('b', '1'); localStorage.setItem('a', '2'); \
             localStorage.setItem('c', '3'); localStorage.removeItem('c'); \
             localStorage.setItem('gone', 'x'); localStorage.setItem('b', 'one'); \
             sessionStorage.setItem('s', '1');</script>",
        )),
        "/clear.html" => Some(html(
            "<script>localStorage.clear(); localStorage.setItem('after', '1');</script>",
        )),
        "/add.html" => Some(html("<script>localStorage.setItem('z', '1');</script>")),
        "/read.html" => Some(html(
            "<script>const keys = []; for (let i = 0; i < localStorage.length; i++) \
             keys.push(localStorage.key(i) + '=' + localStorage.getItem(localStorage.key(i))); \
             console.log(keys.join(' '), sessionStorage.length);</script>",
        )),
        _ => None,
    });
    let memory = Memory::new();
    let mut tab = scripted(Some(&memory));
    load(&mut tab, server.url("/set.html"));
    load(&mut tab, server.url("/read.html"));
    assert_eq!(console(&mut tab), ["b=one a=2 gone=x 1"]);
    drop(tab);
    let mut tab = scripted(Some(&memory));
    load(&mut tab, server.url("/read.html"));
    assert_eq!(console(&mut tab), ["b=one a=2 gone=x 0"]);
    // An item set after the profile was read comes after its own.
    load(&mut tab, server.url("/add.html"));
    load(&mut tab, server.url("/read.html"));
    assert_eq!(console(&mut tab), ["b=one a=2 gone=x z=1 0"]);
    load(&mut tab, server.url("/clear.html"));
    drop(tab);
    let mut tab = scripted(Some(&memory));
    load(&mut tab, server.url("/read.html"));
    assert_eq!(console(&mut tab), ["after=1 0"]);
    drop(tab);
    // An engine with no profile keeps nothing; given one, it reads the
    // profile's areas in place of what it had.
    let mut tab = scripted(None);
    load(&mut tab, server.url("/read.html"));
    assert_eq!(console(&mut tab), [" 0"]);
    load(&mut tab, server.url("/set.html"));
    tab.engine
        .set_profile(Profile::in_memory(&memory).unwrap())
        .unwrap();
    load(&mut tab, server.url("/read.html"));
    assert_eq!(console(&mut tab), ["after=1 1"]);
}

#[test]
fn a_change_is_written_when_its_interval_has_passed() {
    let server = page("ls.setItem('k', 'v');");
    let memory = Memory::new();
    let mut tab = scripted(Some(&memory));
    tab.hand_clock();
    tab.engine.flush_profile().unwrap();
    load(&mut tab, server.url("/page.html"));
    let written = |memory: &Memory| {
        Profile::in_memory(&memory.snapshot())
            .unwrap()
            .local_storage(&server.url("/").origin().serialization())
            .unwrap()
            .len()
    };
    assert_eq!(written(&memory), 0);
    assert_eq!(tab.engine.next_task_at(), Some(Duration::ZERO));
    tab.engine.run_tasks();
    assert_eq!(written(&memory), 1);
    assert_eq!(tab.engine.next_task_at(), None);
}

/// A server whose `/a.html` logs each `storage` event it hears, and
/// whose `/b.html` changes the areas in its head and then waits, so
/// that it is on its way while `/a.html` is shown.
fn event_server() -> TestServer {
    let head = "<!DOCTYPE html><title>b</title><script>\
                addEventListener('storage', e => console.log('b heard ' + e.key + ' ' + \
                e.url.split('/').pop())); \
                localStorage.setItem('k', 'new'); localStorage.setItem('k', 'new'); \
                sessionStorage.setItem('s', '1'); localStorage.removeItem('k'); \
                localStorage.setItem('x', '1'); localStorage.clear(); \
                localStorage.clear();</script><!--";
    let first = format!("{head}{}-->", ".".repeat(1600));
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{first}",
        first.len() + "<p>b</p>".len()
    );
    let a = "<script>localStorage.setItem('k', 'old'); \
             addEventListener('storage', e => console.log([e.key, e.oldValue, e.newValue, \
             e.url.split('/').pop(), e.storageArea === localStorage, \
             e.storageArea === sessionStorage, e.isTrusted, e.bubbles, \
             e.target === window].join(' ')));</script>";
    serve(move |path| match path {
        "/a.html" => Some(html(a)),
        // Which changes its area when its timer runs.
        "/a-timer.html" => Some(html(&format!(
            "{a}<script>setTimeout(() => localStorage.setItem('t', '1'), 100)</script>"
        ))),
        "/b.html" => Some(Reply::Pieces(vec![
            (Duration::ZERO, response.clone().into_bytes()),
            (WAIT, b"<p>b</p>".to_vec()),
        ])),
        _ => None,
    })
}

#[test]
fn a_change_is_heard_by_the_other_document_of_the_origin_alone() {
    let server = event_server();
    let mut tab = scripted(None);
    load(&mut tab, server.url("/a.html"));
    assert!(console(&mut tab).is_empty());
    // Its own change heard by none.
    tab.engine.queue_storage_events();
    assert!(tab.engine.storage_events.is_empty());
    tab.engine.navigate(server.url("/b.html"));
    tab.wait_until(|engine| {
        let storage = engine.storage();
        storage.has_changes(engine.listener)
    });
    // Queued, not fired, until the tasks run.
    assert!(console(&mut tab).is_empty());
    tab.engine.run_tasks();
    assert_eq!(
        console(&mut tab),
        [
            "k old new b.html true false true false true",
            "s  1 b.html false true true false true",
            "k new  b.html true false true false true",
            "x  1 b.html true false true false true",
            "   b.html true false true false true",
        ]
    );
    assert_eq!(tab.engine.next_task_at(), None);
    // The document on its way heard none of its own.
    assert!(incoming_console(&mut tab).is_empty());
}

/// The console lines of the document on its way, taken.
fn incoming_console(tab: &mut Tab) -> Vec<String> {
    match &mut tab.engine.navigation {
        Some(crate::Navigation {
            incoming: crate::loader::Incoming::Parsing { parsing, .. },
            ..
        }) => parsing
            .scripts
            .take_console()
            .into_iter()
            .map(|entry| entry.text)
            .collect(),
        _ => Vec::new(),
    }
}

#[test]
fn the_document_on_its_way_hears_the_one_shown() {
    let server = event_server();
    let mut tab = scripted(None);
    let clock = tab.hand_clock();
    load(&mut tab, server.url("/a-timer.html"));
    tab.engine.queue_storage_events();
    tab.engine.navigate(server.url("/b.html"));
    tab.wait_until(|engine| engine.storage().has_changes(engine.listener));
    tab.engine.run_tasks();
    assert_eq!(console(&mut tab).len(), 5);
    assert!(incoming_console(&mut tab).is_empty());
    // The shown document's timer changes its area: the other hears it.
    clock.advance(100);
    tab.engine.run_tasks();
    assert_eq!(incoming_console(&mut tab), ["b heard t a-timer.html"]);
    assert!(console(&mut tab).is_empty());
}

#[test]
fn a_change_with_no_other_document_of_the_origin_is_heard_by_none() {
    let server = event_server();
    let other = serve(|path| match path {
        "/c.html" => Some(html("<script>localStorage.setItem('c', '1')</script>")),
        _ => None,
    });
    let mut tab = scripted(None);
    load(&mut tab, server.url("/a.html"));
    // Another origin's change, on its way while `/a.html` is shown.
    tab.engine.navigate(other.url("/c.html"));
    tab.wait_until(|engine| incoming(engine).is_none());
    tab.engine.queue_storage_events();
    assert!(tab.engine.storage_events.is_empty());
}
