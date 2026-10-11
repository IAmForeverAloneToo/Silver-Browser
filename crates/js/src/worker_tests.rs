//! A dedicated worker's realm (M31, HTML section 10.2): its global as
//! the reference browser has it (scratchpad m31 `w-scope.js`, Chrome
//! 154), the window's interfaces left out, and the stop flag that
//! `terminate()` sets.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use crate::tests::{Recorder, logged, script};
use crate::{ErrorKind, Limits, QuickJs, Realm, RealmKind, ScriptEngine};

fn worker(stop: Option<Arc<AtomicBool>>) -> Box<dyn Realm<Recorder>> {
    let kind = RealmKind::DedicatedWorker {
        name: "probe".to_owned(),
    };
    QuickJs
        .realm_of(kind, Limits::worker(), stop)
        .expect("a realm")
}

/// The console's lines after running `source` in a worker's realm.
fn in_worker(source: &str) -> Vec<String> {
    let mut realm = worker(None);
    let mut host = Recorder::default();
    realm.run(&script(source), &mut host).expect("it runs");
    assert!(realm.checkpoint(&mut host).is_empty());
    host.console.into_iter().map(|(_, text)| text).collect()
}

#[track_caller]
fn check(lines: &[String], want: &[&str]) {
    for line in want {
        assert!(lines.iter().any(|l| l == line), "no {line:?} in {lines:#?}");
    }
}

/// The probe's look at the global, as `w-scope.js` takes it.
const SCOPE: &str = r#"
var chain = [];
for (var p = Object.getPrototypeOf(self); p; p = Object.getPrototypeOf(p)) {
  chain.push(p.constructor ? p.constructor.name : "?");
}
console.log("self=" + String(self));
console.log("chain=" + chain.join(","));
var names = ["fetch", "XMLHttpRequest", "OffscreenCanvas", "structuredClone", "WebAssembly",
  "document", "window", "Blob", "File", "URLSearchParams", "ImageData", "Path2D",
  "createImageBitmap", "queueMicrotask", "reportError", "setTimeout", "setInterval", "atob",
  "btoa", "performance", "name", "DOMException", "Event", "EventTarget", "MessageEvent",
  "ErrorEvent", "CustomEvent", "DOMMatrix", "DOMPoint", "ImageBitmap",
  "OffscreenCanvasRenderingContext2D", "CanvasGradient", "CanvasPattern", "TextMetrics",
  "FormData", "Headers", "Request", "Response", "AbortController", "localStorage",
  "sessionStorage", "onmessage", "onmessageerror", "onerror", "onlanguagechange", "onoffline",
  "ononline", "onrejectionhandled", "onunhandledrejection", "WorkerGlobalScope",
  "DedicatedWorkerGlobalScope", "SharedArrayBuffer", "alert", "print", "getComputedStyle",
  "HTMLElement", "Node", "Image", "Window", "Location", "Navigator", "Storage", "UIEvent",
  "MouseEvent", "CanvasRenderingContext2D", "HTMLCanvasElement", "importScripts", "location",
  "navigator", "WorkerLocation", "WorkerNavigator", "Worker", "postMessage", "close",
  "MessageChannel", "MessagePort"];
for (var i = 0; i < names.length; i++) {
  console.log("typeof " + names[i] + "=" + typeof self[names[i]]);
}
var where = function (n) {
  var o = self;
  var d;
  while (o && !(d = Object.getOwnPropertyDescriptor(o, n))) {
    o = Object.getPrototypeOf(o);
  }
  return d ? (o.constructor ? o.constructor.name : "own") + ":" +
    Object.keys(d).sort().join(",") : "none";
};
var own = ["onmessage", "self", "name", "onerror", "performance", "location", "navigator",
  "importScripts", "postMessage", "close"];
for (var j = 0; j < own.length; j++) {
  console.log("where " + own[j] + "=" + where(own[j]));
}
console.log("name=" + self.name);
console.log("self is global=" + (self === globalThis));
try { setTimeout(); } catch (e) { console.log("setTimeout=" + e.message); }
try { structuredClone(self); } catch (e) { console.log("clone self=" + e.message); }
try { new WorkerGlobalScope(); } catch (e) { console.log("construct=" + e.message); }
"#;

#[test]
fn a_workers_global_is_as_the_reference_browser_has_it() {
    let lines = in_worker(SCOPE);
    let mut want = vec![
        "self=[object DedicatedWorkerGlobalScope]",
        "chain=DedicatedWorkerGlobalScope,WorkerGlobalScope,EventTarget,Object",
        "where onmessage=DedicatedWorkerGlobalScope:configurable,enumerable,get,set",
        "where self=WorkerGlobalScope:configurable,enumerable,get,set",
        "where name=DedicatedWorkerGlobalScope:configurable,enumerable,get,set",
        "where onerror=WorkerGlobalScope:configurable,enumerable,get,set",
        "where performance=WorkerGlobalScope:configurable,enumerable,get,set",
        "where location=WorkerGlobalScope:configurable,enumerable,get,set",
        "where navigator=WorkerGlobalScope:configurable,enumerable,get,set",
        "where importScripts=WorkerGlobalScope:configurable,enumerable,value,writable",
        "where postMessage=DedicatedWorkerGlobalScope:configurable,enumerable,value,writable",
        "where close=DedicatedWorkerGlobalScope:configurable,enumerable,value,writable",
        "name=probe",
        "self is global=true",
        "setTimeout=Failed to execute 'setTimeout' on 'WorkerGlobalScope': \
         1 argument required, but only 0 present.",
        "clone self=Failed to execute 'structuredClone' on 'WorkerGlobalScope': \
         #<DedicatedWorkerGlobalScope> could not be cloned.",
        "construct=Failed to construct 'WorkerGlobalScope': Illegal constructor",
    ];
    let typeofs = [
        ("fetch", "function"),
        ("XMLHttpRequest", "function"),
        ("OffscreenCanvas", "function"),
        ("structuredClone", "function"),
        ("WebAssembly", "object"),
        ("document", "undefined"),
        ("window", "undefined"),
        ("Blob", "function"),
        ("File", "function"),
        ("URLSearchParams", "function"),
        ("ImageData", "function"),
        ("Path2D", "function"),
        ("createImageBitmap", "function"),
        ("queueMicrotask", "function"),
        ("reportError", "function"),
        ("setTimeout", "function"),
        ("setInterval", "function"),
        ("atob", "function"),
        ("btoa", "function"),
        ("performance", "object"),
        ("name", "string"),
        ("DOMException", "function"),
        ("Event", "function"),
        ("EventTarget", "function"),
        ("MessageEvent", "function"),
        ("ErrorEvent", "function"),
        ("CustomEvent", "function"),
        ("DOMMatrix", "function"),
        ("DOMPoint", "function"),
        ("ImageBitmap", "function"),
        ("OffscreenCanvasRenderingContext2D", "function"),
        ("CanvasGradient", "function"),
        ("CanvasPattern", "function"),
        ("TextMetrics", "function"),
        ("FormData", "function"),
        ("Headers", "function"),
        ("Request", "function"),
        ("Response", "function"),
        ("AbortController", "function"),
        ("localStorage", "undefined"),
        ("sessionStorage", "undefined"),
        ("onmessage", "object"),
        ("onmessageerror", "object"),
        ("onerror", "object"),
        ("onlanguagechange", "object"),
        ("onoffline", "undefined"),
        ("ononline", "undefined"),
        ("onrejectionhandled", "object"),
        ("onunhandledrejection", "object"),
        ("WorkerGlobalScope", "function"),
        ("DedicatedWorkerGlobalScope", "function"),
        ("SharedArrayBuffer", "undefined"),
        ("alert", "undefined"),
        ("print", "undefined"),
        ("getComputedStyle", "undefined"),
        ("HTMLElement", "undefined"),
        ("Node", "undefined"),
        // The window's own, which the probe did not ask about.
        ("Image", "undefined"),
        ("Window", "undefined"),
        ("Location", "undefined"),
        ("Navigator", "undefined"),
        ("Storage", "undefined"),
        ("UIEvent", "undefined"),
        ("MouseEvent", "undefined"),
        ("CanvasRenderingContext2D", "undefined"),
        ("HTMLCanvasElement", "undefined"),
        ("importScripts", "function"),
        ("location", "object"),
        ("navigator", "object"),
        ("WorkerLocation", "function"),
        ("WorkerNavigator", "function"),
        ("Worker", "function"),
        ("postMessage", "function"),
        ("close", "function"),
        ("MessageChannel", "function"),
        ("MessagePort", "function"),
    ];
    let typeofs: Vec<String> = typeofs
        .iter()
        .map(|(name, kind)| format!("typeof {name}={kind}"))
        .collect();
    want.extend(typeofs.iter().map(String::as_str));
    check(&lines, &want);
}

#[test]
fn a_windows_global_has_no_workers_interfaces() {
    let lines = logged(
        "console.log(typeof WorkerGlobalScope, typeof DedicatedWorkerGlobalScope, \
         self === window, String(self), typeof document, typeof Image, \
         typeof WorkerLocation, typeof importScripts, typeof Worker)",
    );
    assert_eq!(
        lines,
        ["undefined undefined true [object Window] object function undefined undefined function"]
    );
}

#[test]
fn a_workers_realm_has_no_step_budget_but_stops_when_told() {
    assert_eq!(Limits::worker().steps, u32::MAX);
    let stop = Arc::new(AtomicBool::new(false));
    let mut realm = worker(Some(stop.clone()));
    let mut host = Recorder::default();
    let setter = {
        let stop = stop.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            stop.store(true, Ordering::Relaxed);
        })
    };
    let error = realm
        .run(&script("for (var i = 0; i < 1e8; i++) {}"), &mut host)
        .expect_err("it is stopped");
    assert_eq!(error.kind, ErrorKind::Budget);
    setter.join().expect("the setter ends");
}
