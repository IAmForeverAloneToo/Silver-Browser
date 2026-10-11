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
worker (22.9 ms at M30, M35.12's to cut).

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
beside the string as attachments the receiver's host takes. A
transferred `ArrayBuffer` is detached after the serialization, as the
standard says.

**Ports, and the router (M31.1, M31.5's channel).** Every pair of
entangled ports, a `Worker` object and its worker's global included
(their implicit ports), is known to one router the engine owns and
shares with its workers (`Arc<Mutex<Router>>`): a port's number, its
entangled port's, and which realm's inbox holds it now. Posting to a
port hands the router the message for the entangled port's holder; a
port in transit (in a message not yet received) has its messages kept
by the router until the realm receiving it adopts it. In the realm, a
`MessagePort` keeps its messages until it is started (`start()` or
`onmessage`), as the reference browser does; closing it unregisters
it. `MessageChannel` makes a pair; `window.postMessage` posts to the
window's own realm, with the target origin checked against the page's.

**The `Worker` object and its life (M31.1, M31.2).** `new Worker(url,
options)`, on a page or in a worker: the URL parsed against the
creator's base URL; one of another origin than the creator's refused
with the reference browser's `SecurityError` (a `data:` URL taken,
its worker's origin opaque); `type: "module"` a load failure (module
scripts are G41); the options' `name` and `credentials` read. The host
starts a thread, which fetches the script (destination `worker`, mode
`same-origin`, the creator's referrer and address space, the creator's
credentials mode), refuses what the network refuses or what
`net::mime::blocked_as_script` blocks (a plain `error` event at the
`Worker` object), and runs it; messages posted to the `Worker` before
it runs wait in its port. The worker's event loop takes, in arrival
order, messages, timers, fetch answers and animation frames, with a
microtask checkpoint after each. `close()` sets the closing flag: the
task ends, and nothing more runs. `terminate()` sets the stop flag,
interrupts what runs and drops what is queued. A worker ends with its
owner: a document leaving (navigation, a tab closed) or a worker
ending ends the workers it made. Past `MAX_WORKERS` live workers a
document's new one waits for one to end, which the standard allows.

**Errors (M31.2).** A worker's uncaught error is first reported in the
worker as the page's are (its `error` event, `onerror` with five
arguments, canceling handles it). Unhandled, it crosses to the owner as
an error message, which the owner's loop dispatches as a cancelable
`ErrorEvent` at the `Worker` object, "Uncaught " and the error's
message, the script's URL, line and column; not canceled there, it is
reported in the owner's realm as an error of its own (the window's
`error` event, then the console's "Uncaught ... at url:line:col"). A
worker's console messages go to its document's console, as the
reference browser's devtools show them beside the page's.

**The worker's scope (M31.1, M31.2).** What the allowlist exposes
works through the worker's host, which answers the same calls the
document's host does where they have a meaning without a document:
the timers (on the worker's own clock, real time), `fetch` and
`XMLHttpRequest` (the worker's own pool of fetch threads, the page's
client, cookies and cache, the worker's URL as the request's client),
`importScripts` (each script fetched synchronously and run in order,
with the reference browser's `NetworkError` for one that fails or
whose type is not a script's), `location` and `navigator`
(`WorkerLocation`, `WorkerNavigator`), `performance.now()` from the
worker's own time origin, `atob`, `btoa`, `structuredClone`,
`reportError`, `queueMicrotask`, WebAssembly (M29's bridge, per
realm), the `OffscreenCanvas` context and `ImageBitmap` (a
`Canvases` of the worker's own), and nested workers.

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
asked for.

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
`DataCloneError`. A worker's heap is the
page's (`js::MAX_SCRIPT_HEAP`), its stack the engine's.

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
or makes values, never panics, never loops.

**The web-platform tests.** `workers` (dedicated workers' tests:
`constructors`, `semantics`, `interfaces` and the directory's own),
`webmessaging` (without frames, by the rule) and the structured-clone
tests of `html/infrastructure/safe-passing-of-structured-data` and
`html/webappapis/structured-clone`, by the pull's rule; and the worker
variant of every `.any.js` test the subset already takes, which the
runner now makes as wptserve does (`.any.worker.html`, the test run in
a worker by `fetch_tests_from_worker`). A test that needs a shared or
service worker, a module worker, a `blob:` URL or a frame is left out
by the rule.

**Golden case** 132: a page whose canvas a worker draws through
`transferControlToOffscreen`, captured from the reference browser.

**Acceptance (M31.5).** The roadmap's: a Mandelbrot page that hands
its rows to four workers, which compute and send them back as
transferred buffers the page puts on its canvas, while an animation of
its own goes on: in the engine and the real shell, every frame of the
page within the frame budget while the workers compute, and the set
drawn as the golden page's; `benches/workers.rs` measuring a worker's
start, a message's round trip, a transfer of 4 MB and the Mandelbrot
page with and without workers. T4.3 and fuzz runs.

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
