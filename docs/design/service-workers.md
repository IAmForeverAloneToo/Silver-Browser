# Design note: service workers and the Cache API

Roadmap M39. Written before the code, as the record of the decisions,
and corrected to the code when the milestone ends: where they
disagree, the code wins, and section 4 lists the places, marked
*(corrected)*. What ships is described in `docs/architecture.md`
"Current state", and the limits it adds live in `docs/security.md`.

A page here works while its server answers it. Its scripts run in its
document's realm and in dedicated workers (M31, `docs/design/workers.md`),
threads of the document's renderer that end with it; its requests go
from the engine's `Network`, or a worker's own pool, through a `Fetcher`
to `net::Client`, in the browser's network service once there are
processes (M34, `docs/design/processes.md`); what it keeps is Web
Storage and IndexedDB (M28, M36), each by its storage key, in the
profile. There is no `navigator.serviceWorker`, no `caches`, and
nothing between a request and the network that a page's own script
could answer. A site that is meant to open with no network, or to
answer its requests from what it saved, does not.

This milestone gives the engine the service worker: a script a site
registers for a scope of its URLs, which the browser keeps across runs,
installs and activates as the standard's lifecycle says, and starts
when one of the scope's pages, or a page's request, needs it; which
answers the scope's navigations and their documents' requests with
responses of its own, from the network, or from the Cache API's
caches, that a site fills itself and the browser keeps. Its acceptance
is the roadmap's "offline": a site whose worker kept its pages at
install opens with its server stopped, in another tab and after the
profile is closed and opened again.

The documents this note follows, at the versions current when it was
written (22 October 2026):

- Service Workers (W3C Editor's Draft of October 2026): the
  `ServiceWorker`, `ServiceWorkerRegistration`, `ServiceWorkerContainer`
  and `ServiceWorkerGlobalScope` interfaces, `Client`, `WindowClient`
  and `Clients`, `ExtendableEvent`, `InstallEvent`, `FetchEvent` and
  `ExtendableMessageEvent`, the `CacheStorage` and `Cache` interfaces,
  and the algorithms: the jobs (Register, Update, Install, Activate,
  Unregister), Soft Update, Handle Fetch, Match Service Worker
  Registration, the Query Cache and Batch Cache Operations;
- the Fetch Standard: the service worker in "main fetch" and "HTTP
  fetch" (a request's service-workers mode, its client and reserved
  client, a response's service worker timing), the `serviceworker`
  destination, the `Service-Worker` request header;
- the HTML Standard: environment settings objects and their ids (the
  client a request is made for), secure contexts, the navigation
  algorithm's reserved environment;
- Storage: the storage key (the origin and its top-level site, M33.2)
  that registrations and caches are kept by, and `estimate()`.

What the standards leave open, or say but are worth checking, was put
to the reference browser (Chrome 155.0.8059.85, headless) before this
note was written, its pages served on two origins of `127.0.0.1`
(scratchpad `m39/pages`, `p1` to `p6`; the page posts what it found to
the probe's server, Chrome 155 writing `--dump-dom`'s output nowhere a
pipe reads):

- *The lifecycle.* `register()` resolves with the registration, its
  `scope` the script's directory by default, `updateViaCache`
  `"imports"`, its `installing` worker in state `installing`; a second
  `register()` of the same script and scope, and `getRegistration()`,
  give the same object. The worker sees `install` (an `InstallEvent`,
  `registration.installing` itself), its `waitUntil` holding it;
  `statechange` follows on the page's `ServiceWorker`: `installed`,
  `activating`, `activated`. `ready` resolves once the registration has
  an active worker, already in `activating`; `controller` stays `null`
  in the page that registered until a reload (or `clients.claim()`),
  then is the active worker itself, `controller === reg.active`. The
  global is a `ServiceWorkerGlobalScope` with `clients`,
  `registration`, `skipWaiting`, `caches`, `serviceWorker`,
  `importScripts` and `fetch`; `activate` is a plain
  `ExtendableEvent`. A worker with no `fetch` listener is passed over
  for its scope's requests: they go to the network unseen.
- *Fetch.* A request a controlled page makes is a `FetchEvent` at the
  worker, its `request` the page's (mode `cors` for `fetch()`, `no-cors`
  and destination `script` or `image` for the parser's, `navigate` and
  `document` or `iframe` for navigations), `clientId` the page's and,
  for a navigation, `resultingClientId` the document to be. A response
  of the worker's own, `new Response(...)`, reaches the page as type
  `basic`, its URL the request's, its status, status text and fields as
  made; one from `fetch(event.request)` as the network's.
  `Response.error()` and a rejected promise are "TypeError: Failed to
  fetch". With no `respondWith` the request goes to the network.
  `respondWith` twice throws `InvalidStateError` "Failed to execute
  'respondWith' on 'FetchEvent': respondWith() was already called.",
  after the event's handlers ended "... The event handler is already
  finished." and the request goes to the network. `Response.redirect`
  is followed (`redirected` true, the final URL). An opaque response
  answering a `cors` request is a network error; answering a `no-cors`
  one it is opaque, status 0. A frame of a controlled page navigates
  through the worker and is controlled. A request's method and body
  reach the worker (`POST`).
- *The Cache API.* `caches.keys()` lists caches in the order they were
  made; `open()` of an existing name resolves to a new `Cache` object
  over the same entries. `put()` resolves with `undefined`; a key is a
  URL or a `Request`; `keys()` and `matchAll()` give entries in the
  order they were put, an entry replaced moving to the end. `match()`
  honours `ignoreSearch`, `ignoreMethod` (else only `GET` matches) and
  `ignoreVary` (else the request's fields that the response's `Vary`
  names must be equal); a response made by script comes back with an
  empty `url` and type `default`, one fetched as it was (`basic`,
  `opaque`). Each match is a new `Response`, its body unread.
  `caches.match()` looks through the caches in their order; a deleted
  cache's `Cache` object still answers from what it held. The errors
  are `TypeError`s worded "Failed to execute 'put' on 'Cache': " then
  "Request method 'POST' is unsupported", "Response body is already
  used", "Partial response (status code 206) is unsupported", "Vary
  header contains *", "Request scheme 'data' is unsupported";
  `add()` of a response not ok is "Failed to execute 'add' on 'Cache':
  Request failed", as is `add()` of an opaque response; `addAll()` of
  one request twice is an `InvalidStateError` "Cache.addAll():
  duplicate requests (<URL>)". A `404` may be `put()`. Keys may be of
  another origin; a fragment is kept in the key's URL and ignored by
  `match()`. An opaque response counts toward `estimate()`'s
  `usageDetails.caches` as some 8 MB: the reference pads opaque
  responses so their size does not leak.
- *Registration's errors.* All are rejections, the message beginning
  "Failed to register a ServiceWorker for scope ('<scope>') with script
  ('<script>'): " then: "The path of the provided scope ('/p4/') is not
  under the max scope allowed ('/p4/sub/'). Adjust the scope, move the
  Service Worker script, or use the Service-Worker-Allowed HTTP header
  to allow the scope." (`SecurityError`); "The script has an
  unsupported MIME type ('text/plain')." (`SecurityError`, `text/html`
  too); "A bad HTTP response code (404) was received when fetching the
  script." (`TypeError`); "ServiceWorker script evaluation failed"
  (`TypeError`, a syntax error and an uncaught throw alike). Before a
  script is fetched: "Failed to register a ServiceWorker: The origin of
  the provided scriptURL ('<origin>') does not match the current origin
  ('<origin>')." (`SecurityError`), the same of the scope; "The URL
  protocol of the script ('data:text/javascript,1') is not supported."
  (`TypeError`); "The provided scope ('...a%2fb/') or scriptURL ('...')
  includes a disallowed escape character." (`TypeError`). A
  `Service-Worker-Allowed` field widens the scope. `getRegistration()`
  gives the registration whose scope is the longest prefix of the URL,
  `undefined` for none, a `SecurityError` for another origin's URL;
  `unregister()` resolves `true`, then `false`.
- *Updates.* `update()` refetches the script; bytes that differ make a
  new worker, `updatefound` fired, which installs and waits (`waiting`,
  state `installed`) while the old one controls; `skipWaiting()`
  activates it, and with `clients.claim()` the page's
  `controllerchange` fires and the new worker answers. Registering
  another script for the scope makes a new worker the same way.
- *Where it is.* A sandboxed frame without `allow-same-origin` throws a
  `SecurityError` reading `navigator.serviceWorker` ("Failed to read the
  'serviceWorker' property from 'Navigator': Service worker is disabled
  because the context is sandboxed and lacks the 'allow-same-origin'
  flag.") and `caches` ("... Cache storage is disabled because ...");
  an insecure context has neither (`undefined`, and `caches` is not
  defined at all); a cross-origin frame has both, kept apart by its
  storage key. `clients.matchAll()` gives `WindowClient`s with `type`
  `window`, `frameType` `top-level`, `url`, `visibilityState` and
  `focused`; `clients.get(id)` the same client; `openWindow()` with no
  user activation is refused, `InvalidAccessError` "Not allowed to open
  a window.".

## 1. Decisions

### Where a service worker runs

**A thread in a renderer of its site, owned by the browser (D35).** A
service worker belongs to a registration, which belongs to a profile:
the tabs of a context share it, and it outlives each of them. Renderers
are each tab's own (M37, `docs/design/chrome.md`), so a service worker
gets a renderer of its own for its site, a *worker renderer*, started by
the context when one of the site's workers must run, locked to the
site as any renderer is, sandboxed as one, and ended once no worker of
the site has run for `WORKER_RENDERER_IDLE` (30 seconds). It counts
toward `MAX_ALL_RENDERERS`. In it, each running version is a thread
with a realm of `RealmKind::ServiceWorker`, built on M31's worker
machinery: the same event loop, timers, `fetch`, `importScripts`,
WebAssembly and `MessagePort`s. In one process (the build feature, the
engine's tests) the threads run in the process itself, behind the
same trait (`ServiceWorkerHost`) the browser starts them through, as
`ThreadLauncher` stands for a renderer's process today.

**A version runs while it is needed.** It is started for an event
(`install`, `activate`, `fetch`, `message`) and stopped after
`SERVICE_WORKER_IDLE` (30 seconds, the reference browser's) with no
event pending; an event's `waitUntil` and `respondWith` promises keep
it, up to `SERVICE_WORKER_EVENT_LIMIT` (5 minutes) an event, past which
the version is stopped and the event fails. Its global's state does not
survive a stop, as the standard means: what a worker keeps, it keeps in
the Cache API or IndexedDB.

### The registry

**In the context, kept in the profile.** `engine::service_worker::Registry`
lives in `Context` beside the jar, the cache and the storage, so in the
browser once there are processes: registrations by storage key and
scope, each with its `installing`, `waiting` and `active` versions (the
script URL, its bytes and those of its imported scripts, whether it has
a `fetch` listener, its state), `updateViaCache`, and the time of its
last update check. The jobs run as the standard writes them, one queue
for each scope: Register, Update, Install, Activate, Unregister, with
Soft Update after a navigation it controls when the last check is more
than a day old. A registration and its active version are written to
the profile (`SCHEMA` 6, sealed as every value, the scripts with
them), so a worker is there when the browser starts again; a version
installing or waiting is not kept.

**The rules a register job holds.** The script and the scope are of
the registering document's origin and `http(s)`; neither path may hold
`%2f` or `%5c`; the scope must be under the script's directory unless
the script's response's `Service-Worker-Allowed` field allows it. The
script is fetched by the browser with destination `serviceworker`, the
`Service-Worker: script` field, same-origin credentials, redirects an
error, past the HTTP cache unless `updateViaCache` says otherwise and
then only when younger than a day; its type must be a JavaScript MIME
type, its status ok. A version is new when its script's or an imported
script's bytes differ from the newest one's. Evaluation runs the
script in a version that receives no event; one that throws, or does
not evaluate, rejects the job with "ServiceWorker script evaluation
failed". The errors' names and words are the reference browser's,
above.

**Who may ask.** Only a secure context whose origin is not opaque has
`navigator.serviceWorker` and `caches`; a sandboxed context without
`allow-same-origin` throws as the reference does. Once there are
processes, the browser takes no renderer's word: a renderer locked to a
site registers, looks up and unregisters only for its site's origins,
each by the storage key the browser computes from its own frame tree,
as Web Storage is checked (M34). A worker renderer answers only for the
versions the browser started in it.

### Clients and messages

**Every document and dedicated worker is a client.** Each has an id, a
UUID made by the browser at the document's creation (a navigation's
reserved client gets it at the request, so `resultingClientId` is the
document's id), and a controller: the registration that answered its
navigation, inherited by its dedicated workers and by `about:blank` and
`srcdoc` frames, or none. `navigator.serviceWorker.controller` is its
active version; `controllerchange` fires when `clients.claim()` or an
activation makes it another. `Clients` gives the versions' scripts the
controlled clients of their origin (`includeUncontrolled` adds the
storage key's others), `get`, `matchAll` with `type`, and `claim`;
`openWindow`, `focus` and `navigate` open, focus or navigate the tab
when the user's activation allows, and are refused otherwise as the
reference refuses them.

**Messages are M31's.** `ServiceWorker.postMessage` and a
`Client.postMessage` carry structured clones and transferred ports,
through the browser between renderers; a page's messages wait until it
calls `startMessages()`, sets `onmessage`, or its document is loaded,
as the standard says. A message to a stopped version starts it.

### Fetch

**The browser's network service is where requests meet a worker.**
Every request a renderer makes reaches it already (M34); a request now
carries its client's id, and the service keeps each client's
controller. Before the network, `Handle Fetch`: a navigation of a
frame or a tab is matched against the registrations of its storage key
(the longest scope that is a prefix of its URL, an active version with
a `fetch` listener); another request goes to its client's controller.
The request is given to the version as a `FetchEvent` (its `request` a
`Request` of the request's method, URL, fields, body, mode,
credentials, cache, redirect, destination and referrer); the worker's
`respondWith` answer comes back as a whole response; no answer, or an
answer after the handlers end, sends the request on to the network as
if no worker were there. Requests a worker makes itself, and requests
whose `service-workers mode` is `none` (the worker's own script, a
`fetch()` from inside the worker), are never given to a worker. In one
process, the engine's `Network`, the workers' pools and the navigation
path ask the context's registry the same way.

**What may answer what.** A response a worker gives is checked as the
standard checks it: a network error for `Response.error()`, a rejected
promise, an opaque response to a request whose mode is not `no-cors`,
an opaque-redirect to one whose redirect mode is not `manual`, a
`cors` response to a `same-origin` request, a response whose body was
used; a `Response.redirect` is followed by the page's fetch. A
response made by script has the request's URL. What a worker answers
is subject to the client's own checks after it (CSP, CORB's and ORB's
filtering of an opaque body, which the network service already applies
to the worker's own fetch; mixed content).

**Bodies are whole.** The engine has no `ReadableStream` (bodies are
bytes in `Response`, M27), so a worker's response is sent when its body
is whole, within `MAX_RESPONSE_BYTES`; a navigation's document is
streamed from there as the network's is. Streams are G-listed.

### The Cache API

**Kept by the browser, by storage key, in the profile.** `CacheStorage`
and `Cache` are in windows, dedicated workers and service workers of
secure contexts. Their entries live in the context's storage, as
IndexedDB's do (M36), reached in one process directly and from a
renderer by `StorageOp::Cache` (requests and responses with their
bytes); written to the profile (`SCHEMA` 6: the caches of each key in
order, each entry's request (method, URL, the fields `Vary` needs) and
response (type, URL list, status, status text, fields), bodies sealed
apart, read when matched), at the storage's flush. The query is the
standard's: by URL without the fragment, `ignoreSearch`,
`ignoreMethod`, `ignoreVary`; a batch's puts and deletes are applied
whole or not at all. A key's caches are within `CACHE_QUOTA` (512 MiB),
an opaque response counted as `OPAQUE_PADDING` (7 MiB) whatever its
size, so that its size does not show in `estimate()`; a put past the
quota is a `QuotaExceededError`. `estimate()` adds the caches to the
usage, with `usageDetails`. Clearing a site (the DevTools storage
panel) clears its caches and unregisters its workers.

### DevTools

The storage panel (M36) lists a site's caches, their entries and their
bodies' sizes, and deletes them; and its service workers, each with
its scope, its versions' scripts and states, and their clients, with
"Update", "Unregister" and "Stop". The network panel marks a response a
worker gave.

### Bounds

`MAX_REGISTRATIONS` (100 a storage key), `MAX_CACHES` (256 a storage
key), `CACHE_QUOTA` (512 MiB a storage key), `OPAQUE_PADDING` (7 MiB),
`MAX_RUNNING_SERVICE_WORKERS` (32 a context; past it, the one idle
longest is stopped), `SERVICE_WORKER_IDLE` (30 s),
`SERVICE_WORKER_EVENT_LIMIT` (5 min), `WORKER_RENDERER_IDLE` (30 s),
`MAX_IMPORTED_SCRIPTS` (64 a version), the scripts' `MAX_SCRIPT_BYTES`,
`MAX_RESPONSE_BYTES` (64 MiB a response a worker gives), and M31's
message bounds.

### Fuzzing

`cache_api` (any sequence of puts, matches, deletes and keys with their
options, against a model of the standard's query, and every entry's
record written and read back the same) and `sw_registry` (any sequence
of register, update, unregister and match jobs over scopes and script
URLs, against the rules above and a model of Match Service Worker
Registration); `ipc_decode`'s seeds gain the new messages.

### The acceptance (M39.5)

A site whose worker keeps its pages (its document, a sheet, a script,
an image) in a cache at install, loaded once; then, the server stopped,
loaded again in another tab, and again after the profile is closed and
opened: shown whole each time, from the worker. Its update: the server
started again with a changed worker, which installs, waits, and takes
over on `skipWaiting`. The web-platform tests' `service-workers`
directory, the tests the engine can run, with expectations. A bench of
a page of twenty resources through a worker that passes every request
to the network, and through one that answers from its cache, beside
the page with no worker; and a worker's start.

## 2. Not in this milestone, said rather than hidden

- *Navigation preload*, *static routing* (`InstallEvent.addRoutes`),
  `ServiceWorkerGlobalScope.cookieStore`, *Background Sync*, *Periodic
  Background Sync*, *Background Fetch*, *Push* and *Notifications*,
  *Payment Handler*: each its own milestone's.
- *Module service workers* (`type: "module"`): module scripts wait for
  G41; registering one is refused with the reference's words.
- *Streams*: a worker's response, and the Cache API's bodies, are whole
  (no `ReadableStream`).
- *Shared workers* (G82) as clients.
- *A service worker for `file:`*, extensions, or the browser's own
  pages: none have one.
- *Storage buckets*, `navigator.storage.persist()` (G148).

## 3. Order of work

1. *The Cache API*: the store in the context, `SCHEMA` 6's tables,
   `CacheStorage` and `Cache` in windows and dedicated workers, the
   renderer's requests, the quota and `estimate()`, the storage panel;
   the `cache_api` fuzz target.
2. *The registry*: registrations and versions, the job queue, the
   script fetch and its rules, the profile; `ServiceWorkerContainer`,
   `ServiceWorkerRegistration` and `ServiceWorker` in documents, their
   states and events; the `sw_registry` fuzz target.
3. *The worker's global and its host*: `ServiceWorkerGlobalScope`,
   `install` and `activate` with `waitUntil`, `skipWaiting`,
   evaluation, the lifetime; in one process, then in a worker renderer.
4. *Clients and messages*: client ids, controllers, `Clients`,
   `postMessage` both ways, `claim` and `controllerchange`.
5. *Subresources*: `FetchEvent`, `respondWith`, the checks, the
   fallback; documents' requests, scripts' and dedicated workers'.
6. *Navigations*: matching, `resultingClientId`, the controlled
   document, Soft Update.
7. *The acceptance* and the benches.
8. *T4.3 and fuzzing.*
9. *Documents.*

## 4. Corrected to the code

Filled in at the last step.
