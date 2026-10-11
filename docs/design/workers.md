# Design note: Web Workers

Roadmap M31. Written before the code, as the record of the decisions,
and corrected to the code when the milestone ends: where they
disagree, the code wins, and the places are marked *(corrected)*. What
ships is described in `docs/architecture.md` "Current state".

A page's scripts run on one thread, the engine's, between its frames:
a script that computes for a second holds the page still for a
second. The web's answer is the dedicated worker: a script started by
`new Worker(url)`, running in a realm of its own on a thread of its
own, which shares nothing with the page but messages, structured
clones of plain data and the objects transferred with them. Here
there is no `Worker`, no `postMessage` and no structured clone at
all: `structuredClone`, `MessageChannel` and `MessageEvent` are
missing too, and a library that schedules its work through a message
channel (React's scheduler does) finds nothing.

This milestone gives the engine dedicated workers: each its own
realm on its own operating-system thread, its script fetched from the
page's origin, with the `DedicatedWorkerGlobalScope` a worker sees
(timers, `fetch` and `XMLHttpRequest`, `importScripts`, WebAssembly,
`OffscreenCanvas`, nested workers); messages both ways as structured
clones, with transfer; `MessageChannel` and `MessagePort` across
realms and `window.postMessage` to the page itself; termination and
the error events that carry a worker's uncaught errors to its owner;
an `OffscreenCanvas` taken from a page's canvas and drawn from a
worker; and the seam that M34's processes will replace. Its
acceptance is the roadmap's: a page whose Mandelbrot set is computed
by workers stays responsive while they work.

The standards are read at the versions current when this note was
written (11 October 2026):

- the HTML Standard (Living Standard of 9 October 2026): 10 (web
  workers: the `Worker` interface, `WorkerGlobalScope`,
  `DedicatedWorkerGlobalScope`, `importScripts`, `WorkerNavigator`,
  `WorkerLocation`, a worker's lifetime and its processing model), 9.4
  (cross-document messaging, for `window.postMessage`), 9.5 (channel
  messaging: `MessageChannel`, `MessagePort` and the port message
  queues), 2.7 (safe passing of structured data: `StructuredSerialize`,
  `StructuredDeserialize`, transferring, `structuredClone`), 8.1.4.6
  (reporting an exception, which for a worker goes on to its `Worker`
  object), 4.12.5.3 (`transferControlToOffscreen` and the placeholder
  canvas) and 8.10 (animation frames, which a dedicated worker has);
- Web IDL for `[Global]`, `[Exposed]`, `[Serializable]` and
  `[Transferable]`;
- the Fetch Standard for a worker's script, its destination `worker`
  and its same-origin mode.

What the standards leave open, or say but are worth checking, was put
to the reference browser (Chrome 154.0.8037.99, headless) before this
note was written, its pages served on two origins (scratchpad
`m31/pages`, `p1-scope` to `p5-offscreen`):

- *The scope.* A worker's global is a `DedicatedWorkerGlobalScope`,
  then `WorkerGlobalScope`, `EventTarget`, `Object`; the dedicated
  interface's members (`name`, `postMessage`, `close`, `onmessage`,
  `onmessageerror`, `requestAnimationFrame`) are the global's own
  properties, `WorkerGlobalScope`'s (`self`, `location`, `navigator`,
  `importScripts`, `onerror`, the timers, `fetch`, `atob`,
  `structuredClone`, `createImageBitmap` ...) its prototype's. There is
  no `document`, `window`, `Node`, `localStorage` or `alert`; there are
  `XMLHttpRequest`, `OffscreenCanvas`, `MessageChannel`, `Worker`
  (nested workers), `WebAssembly` and `ImageData`. `location` is the
  script's URL, `self.origin` its origin, `name` the constructor's
  option.
- *Structured clone.* Values come back as the algorithm says: `-0`,
  `NaN` and BigInts kept; `Number`, `String`, `Boolean` and `BigInt`
  objects as such; a `RegExp` with its flags and `lastIndex` 0; `Map`,
  `Set`, cycles and sparse arrays with their own properties; views over
  one buffer still sharing it; an error with its name among the seven
  and its `message` and `cause`, another name becoming `Error`;
  `DOMException`, `Blob`, `File` and `ImageData`; a class's instance a
  plain object; getters read, in property order (`bac` for `b`, `a`,
  `c`), and a getter's exception thrown. A function, symbol, node,
  promise, `WeakMap`, proxy, window or event is a `DataCloneError`
  (code 25), worded "Failed to execute 'postMessage' on 'Worker':
  function f() {} could not be cloned.", "Symbol(q) could not be
  cloned.", "HTMLBodyElement object could not be cloned.", "#<Promise>
  could not be cloned.".
- *Transfer.* A transferred `ArrayBuffer` is detached (`byteLength` 0)
  and arrives whole; transferring it again is "ArrayBuffer at index 0
  is already detached.", twice in one list "ArrayBuffer at index 1 is
  a duplicate of an earlier ArrayBuffer.", a plain object "Value at
  index 0 does not have a transferable type."; `postMessage(v,
  {transfer})` is the options form; with no argument, "1 argument
  required, but only 0 present.".
- *Errors.* A worker's uncaught error goes first to its own `onerror`,
  called with message, file, line, column and the error (returning
  true handles it) and to its `error` listeners; then, unhandled, to
  an `ErrorEvent` at the `Worker` object, cancelable, its message
  "Uncaught Error: boom", its file the script's URL, its line and
  column the throw's, its `error` null; then, not canceled there, to
  the page's window as an error of the page's own, as the standard
  says. A script that fails to load (404) is a plain `Event` named
  `error`; a syntax error an `ErrorEvent` ("Uncaught SyntaxError:
  ..."). A URL of another origin throws from the constructor: a
  `SecurityError` (code 18), "Failed to construct 'Worker': Script at
  'http://127.0.0.1:8082/p/w-echo.js' cannot be accessed from origin
  'http://127.0.0.1:8081'."; a `data:` URL runs, with the origin
  `null`. A main script served as `text/html` runs; `importScripts` of
  one throws a `NetworkError` (code 19), "Failed to execute
  'importScripts' on 'WorkerGlobalScope': The script at '...' failed to
  load.", as it does for a 404, while a script of another origin is
  imported.
- *Lifetime.* `close()` lets the current task finish (a message posted
  after it in the same task arrives) and runs nothing more: no timer,
  no message. `terminate()` stops the worker at once; nothing it posted
  afterwards arrives, and posting to it is no error.
- *Ports.* A port delivers once started, by `start()` or by setting
  `onmessage`; messages sent before are kept, in order. A closed port
  sends and receives nothing. A port's own transfer is "Port at index
  0 contains the source port.", a port posted but not transferred "A
  MessagePort could not be cloned because it was not transferred.",
  one already moved "Port at index 0 is already neutered.".
  `window.postMessage(m, "*")` delivers to the window itself with the
  page's origin and the window as `source`; a target origin that is not
  the page's is dropped silently, one that does not parse a
  `SyntaxError`.
- *Offscreen.* `transferControlToOffscreen()` gives an
  `OffscreenCanvas` of the canvas's size, refused (`InvalidStateError`)
  on a canvas with a context or once already; the canvas then refuses
  `getContext` and a new `width`. Transferred to a worker, what the
  worker draws shows in the canvas once the worker's task ends (not
  during it); a new width there resizes the canvas. A transferred
  canvas with a context, or one posted without transfer, is refused.

## 1. Decisions

**Threads, and the seam M34 replaces (M31.3).** A worker is an
operating-system thread (`std::thread`, the engine's 32 MiB stack),
made by its owner and running its own event loop until it is closed,
terminated or its owner goes. It shares nothing with any other thread
but values that are `Send`: messages as serialized strings with their
attachments, the network's client, cookie jar and cache (already
behind `Arc` and `Mutex`), and a stop flag. A realm cannot cross
threads anyway: `rquickjs` without its `parallel` feature holds
`Rc`s, and the `js` crate forbids `unsafe`. So the worker's thread
makes its realm itself, which also gives QuickJS the right stack to
measure. Every crossing goes through one type, a `WorkerMessage` on a
channel to the receiver's inbox, followed by a wake: M34's process
model turns exactly these channels into its IPC, the serialized
messages already being bytes.

**A worker's realm (M31.1).** The `js` crate gains the kind of realm
it makes: `RealmKind::Window` as now, or `RealmKind::DedicatedWorker`
with its name. The same bootstrap is evaluated for both, told the
kind: for a worker, `expose()` and the eleven other places that put an
interface on the global take only what Web IDL exposes to a dedicated
worker (an allowlist), and the global is made a
`DedicatedWorkerGlobalScope` (prototype chain and member placement as
the reference browser has them, above) instead of a `Window`; the
timers' and the interfaces' errors name the global's interface. A
worker's `Limits` have no step budget (a worker exists to compute for
long) but a stop flag, an `Arc<AtomicBool>` its interrupt handler
reads, which `terminate()` sets; the heap bound is the page's. The
realm's cost is the page's too: the bootstrap is compiled for each
worker (22.9 ms at M30, M35.12's to cut). *(corrected: `expose()` keeps
every interface in a registry of the bootstrap's own and puts on the
global only those its kind has, the bootstrap reading interfaces from
the registry, never from the global, which a page can change; the
members the window and a worker share name `WorkerGlobalScope` in a
worker's errors, Web IDL's rule for a mixin, not probed.)*

**Structured clone (M31.1).** Written in the bootstrap, where the
values are: `StructuredSerializeWithTransfer` walks a value into a
record of tagged entries (primitives, the wrapper objects, `Date`,
`RegExp`, `ArrayBuffer` and its views by reference to it, `Map`,
`Set`, arrays and plain objects with their own enumerable string keys,
errors, `DOMException`, `Blob` and `File`, `ImageData`, `ImageBitmap`,
and by transfer `MessagePort` and `OffscreenCanvas`), a memory of the
objects met keeping cycles and shared buffers, in the standard's
order and with the reference browser's words for what it refuses.
`StructuredDeserialize` makes the values again in the receiving realm.
Within a realm (`structuredClone`, `window.postMessage` to itself)
the record is used as it is; across realms it crosses as a string,
the record's JSON with the bytes of buffers in base64 (`toBase64`, as
M29's memory crosses), since `rquickjs` lends no buffer's bytes
without `unsafe`. Objects whose state is the engine's (an
`ImageBitmap`'s pixels, a transferred `OffscreenCanvas`, a port) go
beside the string as attachments the receiver's host takes. *(corrected:
there are no attachments; a port and an offscreen canvas cross as
their numbers, which the router and the placeholder registry give
meaning in any realm of the document, and an `ImageBitmap` crossing
realms as its premultiplied pixels in base64 within the string, made
again on arrival. A bitmap in a transfer list is refused: transferring
one is a gap.)* A transferred `ArrayBuffer` is detached after the
serialization, as the standard says. *(corrected: so are the list's
checks of what went before, a detached buffer or a closed port, which
the web-platform tests found made first; a resizable buffer keeps its
bound, and a view out of its buffer's bounds is refused.)*
*(corrected: an error's `stack`, which QuickJS keeps in an accessor of
`Error.prototype`, is read through it and given to the clone as its
own, as V8 has it; a `DOMException`'s clone has none, as the reference
browser's.)*

**Ports, and the router (M31.1, M31.5's channel).** Every pair of
entangled ports is known to one router the document owns and shares
with its workers (`Arc<Mutex<Router>>`): a port's number, its
entangled port's, and which realm holds it now. *(corrected: a
`Worker` object and its worker's global are not implicit ports; they
talk over the worker's channel and its owner's inbox, which is
simpler and as fast.)* Posting to a port hands the router the message
for the entangled port's holder; a port in transit (in a message not
yet received) has its messages kept by the router until the realm
receiving it adopts it. *(corrected: the router, not the realm, keeps
a port's messages until it is started, `start()` or setting
`onmessage`, so that they wait in one place, in transit or not.)*
Closing a port disentangles both. `MessageChannel` makes a pair;
`window.postMessage` posts to the window's own realm, with the target
origin checked against the page's. *(corrected: past `MAX_PORTS` a new
`MessageChannel` is a `RangeError`; a port's messages reach a
document through its inbox, whose wake makes the engine take them,
since a page that schedules work through a channel, as React's does,
has no timer to wake anyone.)*

**The `Worker` object and its life (M31.1, M31.2).** `new Worker(url,
options)`, on a page or in a worker: the URL parsed against the
creator's base URL; one of another origin than the creator's refused
with the reference browser's `SecurityError` (a `data:` URL taken, its
worker's origin opaque); `type: "module"` a load failure (module
scripts are G41); the options' `name` and `credentials` read. The host
starts a thread, which fetches the script (destination `worker`, mode
`same-origin`, the creator's referrer and address space, the creator's
credentials mode) *(corrected: the URL parsed in the document's
encoding, as its other URLs are, and the script decoded as UTF-8
whatever byte order mark it has, as an imported one is)*, refuses what
the network refuses or what `net::mime::blocked_as_script` blocks (a
plain `error` event at the `Worker` object), and runs it; messages
posted to the `Worker` before it runs wait in its port. The worker's
event loop takes, in arrival order, messages, timers, fetch answers
and animation frames, with a microtask checkpoint after each.
`close()` sets the closing flag: the task ends, and nothing more runs.
`terminate()` sets the stop flag, interrupts what runs and drops what
is queued. A worker ends with its owner: a document leaving
(navigation, a tab closed) or a worker ending ends the workers it
made. *(corrected: a message posted to a worker whose thread ended is
dropped, not refused, until its end reaches the page.)* *(corrected: a
document with a worker that has not ended is not kept by the
back-forward cache, so that its workers end as it goes; the reference
browser keeps such a page since 2023, its workers suspended.)* Past
`MAX_WORKERS` live workers a document's new one waits for one to end,
which the standard allows; one past `MAX_WORKER_DEPTH`, like a module
worker, fails to load.

**Errors (M31.2).** A worker's uncaught error is first reported in the
worker as the page's are (its `error` event, `onerror` with five
arguments, canceling handles it). Unhandled, it crosses to the owner as
an error message, which the owner's loop dispatches as a cancelable
`ErrorEvent` at the `Worker` object, "Uncaught " and the error's
message, the script's URL, line and column; not canceled there, it is
reported in the owner's realm as an error of its own (the window's
`error` event, then the console's "Uncaught ... at url:line:col"). A
worker's console messages go to its document's console, as the
reference browser's devtools show them beside the page's. *(corrected:
a script that fails to load, a module worker and one past
`MAX_WORKER_DEPTH` give a plain `error` event, cancelable as the
reference browser's; an unhandled rejection stays in the worker, its
console line the document's; the `Worker` interface has `onerror` and
`onmessage` and no `onmessageerror`, as the reference browser's.)*

**The worker's scope (M31.1, M31.2).** What the allowlist exposes
works through the worker's host, which answers the same calls the
document's host does where they have a meaning without a document:
the timers (on the worker's own clock, real time), `fetch` and
`XMLHttpRequest` (the worker's own pool of fetch threads, the page's
client, cookies and cache, the worker's URL as the request's client),
`importScripts` (each script fetched synchronously and run in order,
with the reference browser's `NetworkError` for one that fails or
whose type is not a script's; *(corrected: every URL is parsed first,
one that does not parse a `SyntaxError`, where the reference browser's
parser takes `http://bad host/` and fails it as a load; a cross-origin
import's errors are not muted)*), `location` and `navigator`
(`WorkerLocation`, `WorkerNavigator`), `performance.now()` from the
worker's own time origin, `atob`, `btoa`, `structuredClone`,
`reportError`, `queueMicrotask`, WebAssembly (M29's bridge, per
realm), the `OffscreenCanvas` context and `ImageBitmap` (a
`Canvases` of the worker's own), and nested workers. *(corrected: a
worker's fetches run on a pool of six threads of its own, which wake
its loop through its inbox; the document's offscreen code was split
out of its host, so that a worker draws with the same code; its
`WorkerNavigator` has the window navigator's members but
`hardwareConcurrency`, `deviceMemory` and the like, which neither has
here; a worker's synchronous `XMLHttpRequest` takes a timeout and a
response type, and it has no `responseXML` or `"document"`.)*

**`OffscreenCanvas` from a page's canvas (M31.4).** The roadmap's
alternative is taken: the software-surface handoff, since the canvas
draws on the processor (D22). `transferControlToOffscreen()` makes the
canvas a placeholder: it refuses a context and a new size, as the
reference browser does, and shows the frames its `OffscreenCanvas`
commits. Drawn on the page's own thread, the offscreen canvas shows
in the placeholder at the next frame. Transferred to a worker, the
worker's host sends the placeholder a frame (the bitmap's snapshot, an
`Arc` of its pixels) after each task that changed it, and the engine
shows it in the placeholder at its next frame, resizing it when the
worker gave the canvas a new size. A dedicated worker has
`requestAnimationFrame`, run on its loop sixty times a second while
asked for. *(corrected: the frames go to the document's realm through
the router, whatever worker the canvas went to; a placeholder's
`toDataURL` encodes its own empty bitmap where the reference browser
encodes the frame.)*

**Limits (the bounds a hostile page cannot pass).**

| Constant | Value | Bounds |
|---|---|---|
| `MAX_WORKERS` | 16 | a document's live workers, nested ones included; past it, new ones wait |
| `MAX_WORKER_DEPTH` | 8 | workers made by workers, from the document |
| `MAX_MESSAGE_BYTES` | 256 MiB | a message's serialized form, attachments' bytes included |
| `MAX_QUEUED_BYTES` | 512 MiB | the messages waiting in one realm's inbox and ports |
| `MAX_PORTS` | 65,536 | a document's ports, its workers' included |

A message past `MAX_MESSAGE_BYTES`, or that would take an inbox past
`MAX_QUEUED_BYTES`, is refused at `postMessage` with a
`DataCloneError` *(corrected: worded as V8's own refusal of a value
too large, "Data cannot be cloned, out of memory.", not probed; the
ports' waiting messages count toward the same bound)*. A worker's
heap is the page's (`js::MAX_SCRIPT_HEAP`), its stack the engine's.

**Tests.** The `js` crate's: a worker realm's global and its members,
the structured clone's values and refusals within a realm against the
reference browser's answers above. The engine's: a page's workers on
the test server, messages both ways and with transfer, errors through
every step, `close` and `terminate`, nested workers, ports between
realms, `importScripts` and `fetch` in a worker, the placeholder
canvas's frames, workers ended with their document.

**Fuzzing.** `dom_api` gains workers: one started from a data URL
with a script that echoes, messages of the values the input chooses,
ports, `terminate`; and a new target, `clone`, which gives the
deserializer the input as a message's string in a realm: it refuses
or makes values, never panics, never loops. *(corrected: the `clone`
target keeps one realm for its inputs, made again when it dies or
fills, since making one is a second under the address sanitizer; it
runs as one process, libFuzzer's fork mode making a realm a job.)*

**The web-platform tests.** `workers` (dedicated workers' tests:
`constructors`, `semantics`, `interfaces` and the directory's own),
`webmessaging` (without frames, by the rule) and the structured-clone
tests of `html/infrastructure/safe-passing-of-structured-data` and
`html/webappapis/structured-clone`, by the pull's rule; and the worker
variant of every `.any.js` test the subset already takes, which the
runner now makes as wptserve does (`.any.worker.html`, the test run in
a worker by `fetch_tests_from_worker`). A test that needs a shared or
service worker, a module worker, a `blob:` URL or a frame is left out
by the rule. *(corrected: in M31's directories a test that makes a
`SharedArrayBuffer` or a `BroadcastChannel` is left out too, a
helper's `SharedArrayBuffer` taken, as `/common/sab.js` makes one only
when asked; the canvas's `.worker.js` tests and `canvas-host` are
taken, a JavaScript test's comment lines not held to the rule. The
tests found ten bugs, among them the list's checks made before the
serialization, a worker's script decoded by its byte order mark, its
URL parsed in UTF-8, a detached buffer's bytes in a `Blob` refused and
a transferred `OffscreenCanvas` drawn; the rest fail by the gaps named,
5,502 of 6,086 tests passing and 517 of the 1,419 expected failures
M31's.)*

**Golden case** 132: a page whose canvas a worker draws through
`transferControlToOffscreen`, captured from the reference browser.
*(corrected: with a canvas the page draws through its own
`OffscreenCanvas` beside it. The worker says it drew from its next
animation frame, since the reference browser commits a worker's canvas
then and not as the worker closes; the page spends real time in
synchronous requests until it hears, the capture's virtual time not
waiting for a worker's thread; and the reference runner takes what a
page's workers send until they are quiet before each step.)*

**Acceptance (M31.5).** The roadmap's: a Mandelbrot page that hands
its rows to four workers, which compute and send them back as
transferred buffers the page puts on its canvas, while an animation of
its own goes on: in the engine and the real shell, every frame of the
page within the frame budget while the workers compute, and the set
drawn as the golden page's; `benches/workers.rs` measuring a worker's
start, a message's round trip, a transfer of 4 MB and the Mandelbrot
page with and without workers. T4.3 and fuzz runs. *(corrected: the set
is compared with the one the page computes on its own thread, pixel
for pixel, golden page 132 being a worker's canvas; the frames are
measured from one to the next at the display's ticks. The real shell
drew a wanted frame as soon as the last was presented, about 270 a
second, a software surface having no vertical sync: it now waits for
a frame's budget after the last.)*

## 2. Not in this milestone, said rather than hidden

- *Shared workers* (`SharedWorker`) and *service workers* (M39).
  A new gap, G82, for shared workers.
- *Module workers* (`type: "module"`): module scripts are G41; such a
  worker fails to load here, where the reference browser runs it.
- *`blob:` URLs* for a worker's script: `URL.createObjectURL` is G68.
- *`SharedArrayBuffer`* and `Atomics.wait` between threads, with
  `crossOriginIsolated` and the headers that make it (COOP and COEP):
  G83. The shared memories of M29's WebAssembly with it.
- *`BroadcastChannel`*: G84.
- *Frames' `postMessage`* (`window.postMessage` to another window):
  M33.2's frames.
- *Transferable streams*: with streams, G67.
- *A worker's own CSP and referrer policy* from its script's response:
  M33.1 and M33.3.
- *Process isolation* of workers: M34, at the seam above.

## 3. Order of work

Each step a vertical slice, with its tests, a sabotage pass and the
gate.

1. *Structured clone in a realm*: the serializer and deserializer in
   the bootstrap, `structuredClone`, `MessageEvent`, and
   `window.postMessage` to the window itself; their tests against the
   reference browser's answers.
2. *A worker's realm*: `RealmKind`, the bootstrap's global scope by
   kind, the worker's limits and stop flag; the `js` crate's tests of a
   worker's global.
3. *Dedicated workers*: the thread and its event loop, the `Worker`
   object, the script's fetch, messages both ways with transfer,
   `close`, `terminate`, errors and the console, nested workers, the
   router's implicit ports, workers ended with their document.
4. *The worker's scope*: timers, `fetch` and `XMLHttpRequest`,
   `importScripts`, `location`, `navigator`, `performance`,
   WebAssembly and canvases in a worker.
5. *`MessageChannel` and `MessagePort`* across realms, with transfer.
6. *`OffscreenCanvas` from a page's canvas*: the placeholder, frames
   from a worker, `requestAnimationFrame` in a worker.
7. *Fuzzing*: the `clone` target, `dom_api`'s workers.
8. *The web-platform tests*: the directories above and the worker
   variants.
9. *Golden case* 132.
10. *Acceptance*: the Mandelbrot page in the engine and the real
    shell, `benches/workers.rs`, T4.3, fuzz runs.
11. *The documents*.
