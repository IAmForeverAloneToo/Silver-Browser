# Design note: storage, the profile and the HTTP cache

Roadmap M28. Written before the code, as the record of the decisions,
and corrected to the code when the milestone ended: where they
disagreed, the code won, and the places are marked *(corrected)*. What
ships is described in `docs/architecture.md` "Current state".

The browser forgets everything when it closes. Its cookie jar lives in
memory (M27), so a site the user signed in to has forgotten them by
the next start. A page's scripts have no storage of their own: there
is no `localStorage` and no `sessionStorage`, and a script that tests
for them finds them missing. Nothing is cached: a page that names the
same image twice is spared the second fetch only by the document's
own image store, and a page loaded again fetches every stylesheet,
script and image again, whatever their `Cache-Control` said. The
browser writes nothing to disk at all but a printed PDF.

This milestone gives the browser a profile, a place on disk for what
the user's browsing keeps, and fills it with the first two things:
the cookies that ask to outlive the session, and each origin's
`localStorage`. It gives each tab its `sessionStorage`, kept in
memory for the tab's life. And it gives `net` an HTTP cache in memory,
which reuses a response while it is fresh as `Cache-Control`,
`Expires`, `Age`, `Date` and `Last-Modified` say. Its acceptance is the
roadmap's: what one origin stores another cannot read, before the
profile is closed and after it is opened again.

The standards are read at the versions current when this note was
written:

- the HTML Standard (Living Standard of 9 October 2026): 12.2 (Web
  Storage: the `Storage` interface, `sessionStorage`, `localStorage`,
  the `storage` event, storage keys and quotas), 7.1.1 (origins);
- the Storage Standard (Living Standard, read 11 October 2026): storage
  keys, buckets and the local and session storage types;
- Web IDL (Living Standard, read 11 October 2026): 3.9 (legacy platform
  objects, named properties, their visibility and the
  `[[GetOwnProperty]]`, `[[Set]]`, `[[DefineOwnProperty]]`,
  `[[Delete]]` and `[[OwnPropertyKeys]]` they have), 2.8.1
  (`QuotaExceededError`);
- HTTP Caching, RFC 9111 (June 2022): 3 (storing responses), 4
  (constructing responses from caches: freshness, its lifetime, the
  current age, the heuristic, invalidation), 5.2 (`Cache-Control`
  directives), 5.3 (`Expires`), 4.1 (`Vary`); and RFC 9110's HTTP-date;
- the Fetch Standard (Living Standard of 6 October 2026): the
  request's cache mode and HTTP-network-or-cache fetch (4.6), which
  says how each mode uses the cache;
- Cookies: HTTP State Management Mechanism,
  draft-ietf-httpbis-rfc6265bis-22: 5.7, whose "persistent-flag" says
  which cookies outlive the session.

What the standards leave to the implementation, or state but are worth
checking, was put to the reference browser (Chrome 154.0.8037.99,
headless) before this note was written, from pages served on two ports
of 127.0.0.1 (scratchpad `m28/probe_server.py`, `pages/p1-storage.html`
to `p4-file.html`, and `probe_persist.py`, which starts the browser
again on one profile directory):

- `localStorage` and `sessionStorage` are the same object at every
  read; `Storage.prototype` has `clear`, `getItem`, `key`, `length`,
  `removeItem`, `setItem`; `new Storage()` is an illegal constructor;
- items are named properties: `ls.c = "named"` stores `"named"`;
  `ls.b` reads an item, `undefined` for none; `"a" in ls` is true;
  `Object.keys(ls)` lists the items; a descriptor is a writable,
  enumerable, configurable data property holding a string;
  `delete ls.b` removes the item and is true, `true` for a missing one
  too; `Object.defineProperty` with a value stores it, with an accessor
  throws `TypeError` ("Accessor properties are not allowed");
  `Object.preventExtensions` throws; a symbol key is an ordinary
  property, not an item;
- an item named as a member of `Storage.prototype` (`getItem`, `key`,
  `length`) is stored and read by `getItem`, while `ls.getItem` stays
  the method and `ls.length` the count; but `ls.key = "y"` makes an own
  property `key` holding `"y"`, which hides the method, rather than
  storing an item, where Web IDL's `[[Set]]` and the web-platform test
  `storage_builtins` store the item and keep the method;
- values are strings: `setItem("b", 1)` reads `"1"`, an object
  `"[object Object]"`, `null` `"null"`, `undefined` `"undefined"`;
  `ls.n = 5` reads `"5"`; a missing argument is a `TypeError`;
- `key(n)` takes an `unsigned long` (`key(-1)` is `null`, `key("1")`
  the second, `key(4294967297)` the second) and gives `null` past the
  end; the order is the browser's own, not the order of insertion (three
  items set as `b`, `a`, `c` come out `a`, `c`, `b`), and keeps when an
  item is replaced;
- a key or value with a lone surrogate keeps it;
- the quota is 5,242,880 UTF-16 code units of keys and values together
  for an origin's `localStorage`, and as many for its `sessionStorage`,
  apart: a key of one unit takes a value of 5,242,879, one of ten
  5,242,870; an accented `e` (U+00E9) counts one unit; replacing an
  item counts only the new value against what the rest holds; past
  it, `setItem` throws `QuotaExceededError` (an interface of its own,
  a `DOMException` of code 22), "Failed to execute 'setItem' on
  'Storage': Setting the value of 'q' exceeded the quota.";
- the `storage` event is not fired at the document whose script
  changed the storage; `StorageEvent`'s defaults are `null` for `key`,
  `oldValue`, `newValue` and `storageArea` and `""` for `url`, and it
  does not bubble; `initStorageEvent` sets them all;
- on a `data:` page reading `localStorage` or `sessionStorage` throws
  `SecurityError` ("Failed to read the 'localStorage' property from
  'Window': Storage is disabled inside 'data:' URLs."); on a `file:`
  page, whose origin serializes as `null`, both work, every `file:`
  page sharing one storage;
- started again on the same profile directory, the browser has kept
  `localStorage` (per origin: two ports of one host do not share it)
  and the cookies with an expiry, set by the server or by a script; it
  has dropped `sessionStorage` and the cookies without one;
- the HTTP cache, each resource fetched three times and the server
  counting what reached it: kept and reused while fresh by `max-age`,
  `Expires`, `private` and `public` with `max-age`, `immutable`, and an
  `Age` under the lifetime; not reused with no freshness information,
  `no-store`, `max-age=0`, `no-cache`, an `Expires` past, an `Age` over
  `max-age`, `s-maxage` alone, `Vary: *`, or for a `POST`; reused with
  `Vary: Accept` when `Accept` was the same; reused by the heuristic
  for a response with only a `Last-Modified` 100,000 s old; a 404 and a
  500 with `max-age` reused; a response with `Set-Cookie` reused; a
  response with an `ETag` and no freshness revalidated each time by a
  conditional request answered 304, its body the cached one;
- by the request's cache mode: `no-store` and `reload` reach the
  server every time, `no-cache` revalidates, `force-cache` reuses even
  what is stale, `only-if-cached` with nothing cached is a network
  error; a CORS response from another origin is cached as well.

## 1. Decisions

**The profile (M28.1, D10).** The user's browsing keeps its data in a
profile: a directory on disk the shell chooses, holding one database
file. The decision recorded as D10: `redb` (4.3.0, MIT or Apache-2.0,
pure Rust, no dependencies, an embedded key-value store of B-trees in
one file, with ACID transactions committed by copy-on-write, crash
recovery on open, and a byte-range lock that keeps a second process
out), rather than SQLite (C, the workspace's first, and `unsafe` at
every call), `sled` (no stable release since 2021) or files of our own
(a format, its atomic replacement and its recovery written and tested
here, and an IndexedDB, M36.4, needing a real store later). `redb` has
a little `unsafe` of its own, a hash's SIMD paths, its lock wrappers
and positioned file reads, which M33.5's audit of third-party code
takes; our code over it writes none.

- *A crate of its own, `profile`*, a leaf beside `encoding`, below
  `engine`, with `#![forbid(unsafe_code)]`: the only crate that knows
  `redb`. It offers what the engine keeps, not a key-value store: an
  origin's `localStorage` items read whole and written whole, and the
  cookies read whole and written whole, each write one transaction.
  Its types are its own (an item a pair of `String`s, a cookie a plain
  record), so that neither `net` nor `engine` depends on `redb`, and
  `profile` depends on neither. *(corrected)* An origin's items are
  read whole but written by the item: a commit's `Changes` set, remove
  and clear an origin's items in the order given, so that a change
  writes what changed. The profile has no cookie type: a cookie is a
  record of `net`'s encoding, which `profile` keeps without reading,
  the cookie being `net`'s type (rule 3); the records are replaced
  whole at each commit that has them. A `Memory`, bytes that outlive
  the profile on them, stands in for the file in tests and fuzzing: a
  profile dropped and opened again, a copy taken as a crash would leave
  it, a byte damaged.
- *Where it lives.* The shell chooses the directory, the only crate
  that may know the platform's places (rule 6): `--profile <dir>`, or
  by default `%LOCALAPPDATA%\SilverBrowser\Profile` on Windows,
  `~/Library/Application Support/SilverBrowser/Profile` on macOS and
  `$XDG_DATA_HOME/silver-browser/profile` (else
  `~/.local/share/silver-browser/profile`) elsewhere. In it, one file,
  `profile.redb`. `--temporary-profile` keeps nothing on disk. The
  engine by itself has no profile, an in-memory one, so that no test
  and no tool touches the user's: `Engine::set_profile` gives it one,
  as `set_network` gives it a client. *(corrected)* With none, the
  engine keeps its areas in memory and writes nothing: there is no
  in-memory profile under it. `--print-to-pdf` keeps nothing unless
  `--profile` names one, as the reference browser's headless mode
  starts on a new profile; `--profile` and `--temporary-profile` are
  one at most.
- *Its tables.* `meta` (the schema's version, 1), `local_storage`
  (keyed by the origin's serialization and the item's key, so that an
  origin's items are one range), `cookies` (keyed by a cookie's domain,
  path and name and whether it is host-only). Strings as UTF-8 bytes; a
  cookie as a record of our own encoding, its fields in order, its
  version first. An origin's namespace is its key's prefix: nothing
  reads across origins but a range scan of one. *(corrected)*
  `local_storage` holds each item's order of first setting with its
  value, so that an area comes back in its order; `cookies` is keyed by
  the record's place in the jar's order.
- *Opening.* A profile that cannot be opened (another process holds
  it, a newer schema, a file `redb` cannot repair) is not used: the
  shell says why and runs on an in-memory profile, which loses nothing
  that was on disk. A profile of an older schema is upgraded in one
  transaction (there is none older yet).
- *Writing.* What scripts and responses change is kept in memory first
  and written in batches: a change marks its origin's storage, or the
  jar, dirty, and the engine writes what is dirty in one transaction at
  most once a second, when its tasks have run, and when it is dropped
  or told to (`Engine::flush_profile`), which the shell does when its
  window closes and before `--print-to-pdf` exits. A crash loses at
  most the last second, as the reference browser's batched commits do.
  Each transaction is durable when it returns (`redb`'s `Immediate`).
  *(corrected)* The write is made from `run_tasks`, which runs a
  second after the last write at the soonest, `next_task_at` saying
  when, so that the shell wakes for it; a write that fails is tried
  again an interval on, and `flush_profile` says why. The shell writes
  after its event loop ends, the window closed.

**Cookies kept (M28.3).** RFC 6265bis's persistent-flag is a cookie
with an expiry (`Expires` or `Max-Age`): those are written to the
profile, and the session's are not, as the reference browser showed.
`net::cookies::Jar` gains `persistent()`, the cookies to keep with
their order of creation and of last use, and `restore`, which takes
them back at startup, drops what has expired, keeps the order and the
limits, and sets the jar's counter past them. The engine loads them
when it is given a profile, and writes the jar when it changed, a
cookie set, replaced, deleted or evicted, on the batches above.
*(corrected)* `Jar::export` gives the cookies with an expiry as records
of `net`'s own encoding (a version, then the name, value, domain and
path each after its length, the flags, `SameSite`, the expiry and the
two orders) and `Jar::restore` takes them back, renumbering both orders
after the jar's own, replacing namesakes, keeping the limits, and
saying whether the profile must be written again. `Jar::kept_changes`
counts the changes to the cookies with an expiry, a cookie sent not
being one. A record is held to the storage model's rules before it is
kept (no control character or `;`, no `=` in a name, what `SameSite`
and the prefixes ask), its expiry to 400 days from now: the profile is
a file anything may have written.

**Web Storage (M28.2).** HTML 12.2, as the bootstrap's `Storage` over
natives, the engine keeping the items:

- *Areas.* An area is an ordered map of keys to values for one origin.
  `localStorage`'s areas are the engine's, one per origin, loaded from
  the profile at their origin's first read and written back when dirty;
  `sessionStorage`'s are the tab's, one per origin, kept by the engine
  in memory: a tab is an engine, so a navigation and a reload keep
  them and nothing else sees them. Both are reached by scripts through
  the host, as `document.cookie` reaches the jar, keyed by the
  document's origin (`Page::origin`), never by what a script says.
  *(corrected)* The engine's storage is shared with a run's host as the
  jar is, through what the window lends (`WindowEnv`); `setItem`,
  `removeItem` and `clear()` note the change for the profile in one
  place (`Storage::set_item`, `remove_item`, `clear_area`), which the
  host and `engine::StorageDriver`, the fuzz target's and the
  benchmark's way in, both call.
- *Opaque origins* have no storage: reading `localStorage` or
  `sessionStorage` on a document whose origin is opaque throws
  `SecurityError`, `data:` pages with the reference browser's words.
  That takes in `file:` pages, which the reference browser gives one
  storage between them all: any local file a user opens could read
  what any other wrote, so they are refused here as HTML's rule says.
  *(corrected)* Their message is the reference browser's for a refusal
  of another kind, "Access is denied for this document."
- *Order.* `key(n)` counts in the order items were first set; an item
  replaced keeps its place, one removed and set again goes last. The
  reference browser's order is its hash map's; the standard lets each
  browser choose, and this one is the deterministic choice.
- *The quota* is the reference browser's: 5,242,880 UTF-16 code units
  of keys and values for each area, counted as UTF-16, an item replaced
  counted by its new value alone. Past it, `setItem` throws
  `QuotaExceededError`, a class of the bootstrap's extending
  `DOMException` (Web IDL's, code 22, `quota` and `requested` `null`),
  in the reference browser's words.
- *Strings.* Keys and values are `DOMString`s, which may hold lone
  surrogates; the engine's strings are Rust's, which cannot, so a lone
  surrogate is stored as U+FFFD, as everything a script hands the
  engine already is. G78.
- *`Storage` itself*, a legacy platform object with named properties
  and a named getter, setter and deleter, is a `Proxy` over a `Storage`
  instance, as `dataset` is: `[[GetOwnProperty]]` and `[[Get]]` see an
  item only where the named property visibility algorithm says (not
  where the object or its prototype chain has a property of that name,
  since `Storage` is not `[LegacyOverrideBuiltIns]`);
  `[[Set]]` with a string key stores the item whatever the prototype
  has, as Web IDL and `storage_builtins` say rather than as the
  reference browser does; `[[DefineOwnProperty]]` stores a data
  property's value and refuses an accessor; `[[Delete]]` removes a
  visible item; `[[OwnPropertyKeys]]` lists the visible items, in
  order, then the target's own keys; `[[PreventExtensions]]` refuses.
  Symbols are ordinary properties of the target. *(corrected)* The
  members are in the reference browser's order (`length`, then the
  operations by name), so that `for`-`in` lists them as it does; the
  named setter's quota message is "Failed to set a named property 'q'
  on 'Storage': ...".
- *Members*: `length`, `key`, `getItem`, `setItem`, `removeItem`,
  `clear`, with Web IDL's conversions (`unsigned long` for `key`,
  `DOMString` for the rest, the argument counts) and the reference
  browser's messages.
- *The `storage` event* (HTML 12.2.1): a change of an area queues, at
  every other document of the same origin alive in the engine that has
  that area, a task firing `StorageEvent` (`key`, `oldValue`,
  `newValue`, `url` the changing document's URL, `storageArea` that
  document's object for the area) at its window. One tab has two such
  documents at most, the one shown and one on its way; tabs (M37)
  bring the rest. `clear` fires one with every field `null`; a change
  that changes nothing fires none. `StorageEvent` and its
  `initStorageEvent` join the event classes. *(corrected)* A change is
  recorded for its event only while the document shown and the one on
  its way share an origin; the engine queues the events and fires them
  from `run_tasks`, one a turn of its loop, at the other document if it
  is still alive, `next_task_at` saying they are due. The engine gives
  the realm the area by a field of its own (`js::Field::Storage`),
  which the bootstrap makes the window's object; `js::Field::Null` is
  a `key` a `clear()` leaves `null`. `clear()` of an empty area fires
  none.

**The HTTP cache (M28.4).** In `net`, a `Cache` the `Client` keeps,
in memory, as RFC 9111 has a private cache, version 0 of it: freshness
only, no revalidation.

- *What is stored.* The response to a `GET` (Fetch's "HTTP-network-
  or-cache fetch" stores and reuses only those here) whose status is
  final and not 206, when neither the request nor the response says
  `no-store`, when `Vary` is not `*`, and when it has an explicit
  lifetime (`max-age`, else `Expires` against `Date`) or is
  heuristically cacheable (a status RFC 9111 4.2.2 names, with a
  `Last-Modified`). `private` responses are stored, this being a
  private cache; `s-maxage` is a shared cache's and not read.
  Navigations' documents are not stored: version 0 is the roadmap's
  subresource cache, and a document reloaded or gone back to is
  fetched again, as now. *(corrected)* The cache is read and written
  in `scheme_fetch` for a `GET` fetched whole: a document, which
  streams, neither reads nor writes, nor does a `HEAD`. Storable means
  `public`, `private`, `max-age`, an `Expires` or a heuristically
  cacheable status (RFC 9110 15.1), whatever its `Last-Modified`; one
  with nothing to give it a lifetime is stored stale, for
  `force-cache`. The cache is one `Arc` an engine's clients share, as
  they share the jar, so that a client given by `set_network` keeps it.
- *Freshness* (RFC 9111 4.2): the lifetime from `max-age`, else
  `Expires` minus `Date`, else 10% of `Date` minus `Last-Modified`, as
  the reference browser's heuristic is; the current age from `Age`,
  `Date` and the times the request was sent and the response came;
  fresh while the lifetime exceeds the age. `no-cache` (and
  `must-revalidate` once stale) means reuse only after revalidating,
  which version 0 does not do: the request goes to the network.
- *Lookup.* Before a hop is sent, by the request's cache key and, for
  a response with `Vary`, the values of the named fields in the request
  as it would be sent. The key: the request's URL without its
  fragment, and the top-level site it is made for (the document's
  site, the URL's own for a request the user made), so that one site
  cannot tell what another's pages fetched by timing a fetch of its
  own: the cache is partitioned by site, as the reference browser's
  has been since 2020. *(corrected)* The site is the request origin's,
  the document's that asked, which with no frames is the top level;
  an opaque origin's site is itself. `max-age` is the first, its
  delta-seconds held to 2^31; an `Expires` that is not a date is in the
  past. The client's clock (`Client::with_clock`) is what ages are
  reckoned by, a hand's in tests. Fetch 4.6's fields are sent:
  `Pragma` and `Cache-Control: no-cache` for `no-store` and `reload`,
  `Cache-Control: max-age=0` for `no-cache`, where the author set none;
  a `default` request with an author's conditional field is
  `no-store`.
- *Cache modes* (Fetch 4.6): `default` uses what is fresh and stores
  what may be; `no-store` neither reads nor writes; `reload` does not
  read, and writes; `no-cache` does not read (revalidation being
  version 0's absence), and writes; `force-cache` uses what is stored,
  fresh or not, and writes; `only-if-cached` uses what is stored and
  is a network error otherwise, in `same-origin` mode only, as the
  `Request` constructor already checks. `Request.cache` sets it now;
  the engine's own subresource fetches take `default`. *(corrected)*
  `only-if-cached` with nothing stored is `NetworkError::NotCached`;
  XHR's requests are `default`; the engine checks `only-if-cached`'s
  mode again as it checks the rest of a script's request.
- *A hit* is delivered as `data:` and `file:` responses are, whole,
  with the hop's URL list; the CORS check, `Cross-Origin-Resource-
  Policy` and the tainting run on it as on a response from the network;
  its `Set-Cookie` is not stored again. `Age` is not rewritten.
- *Invalidation* (RFC 9111 4.4): a response to a request with an
  unsafe method (`POST`, `PUT`, `DELETE`, `PATCH`) that is not an
  error removes what is stored for its URL. *(corrected)* In the
  request's partition alone; a 2xx or 3xx answer is not an error.
- *Bounds.* `MAX_CACHE_BYTES` (64 MiB of bodies in all) and
  `MAX_CACHE_ENTRY_BYTES` (8 MiB, larger responses not stored), the
  least recently used going first; `MAX_CACHE_ENTRIES` (10,000).

**Isolation (M28.5).** What is kept is keyed by the origin that kept
it, and every read is keyed by the reading document's origin from the
engine's side: a script names a key, never an origin. The tests hold
it from both sides: two origins (two ports of `127.0.0.1`, and
`a.localhost` and `b.localhost`) each writing and reading their own
`localStorage`, `sessionStorage` and cookies, and none of the other's,
in the session and after the profile is closed and opened again; an
opaque origin refused; the cache, a resource fetched from one site and
then from another, reaching the server again. *(corrected)* Four
origins, the two hosts and two ports of `127.0.0.1`, on a profile in
memory and on one on disk; cookies are a host's, not an origin's, so
the two ports share theirs, as the reference browser's do, and the
test says so. A tab's `sessionStorage` is its own; `a.localhost` and
`www.a.localhost`, one site, share a cache entry.

**Limits (the bounds a hostile page cannot pass).**

| Constant | Value | Bounds |
|---|---|---|
| `STORAGE_QUOTA` | 5,242,880 units | an area's keys and values |
| `MAX_CACHE_BYTES` | 64 MiB | the HTTP cache's bodies |
| `MAX_CACHE_ENTRY_BYTES` | 8 MiB | a response the cache keeps |
| `MAX_CACHE_ENTRIES` | 10,000 | the HTTP cache's entries |
| `FLUSH_INTERVAL` | 1 s | how long a change waits to be written |

The number of origins with `localStorage` is not bounded beyond what
the disk holds, as in the reference browser; a way for the user to see
and clear them is M36's.

**Tests.** The profile's unit tests run on `redb`'s in-memory backend
and on a temporary directory, opening it again to read what was
written; a profile held by another open is refused and the engine runs
on in-memory storage. The engine's tests load pages from two origins
over the test servers, write, reload and read, and give a second
engine the same profile directory. `net`'s cache tests count what
reaches the server, on a hand clock the cache is given (its times read
through the client's clock, not the system's, so that ages and
lifetimes are tested to the second). *(corrected)* The profile's tests
run on `Memory` rather than `redb`'s own in-memory backend, which
cannot be opened again; Web Storage's are held to the reference
browser's answers to the probe's page, line by line, where Web IDL
does not say otherwise.

**Fuzzing.** A new target, `storage`: operations on a `localStorage`
area and on the profile under it (sets, removals, clears, flushes and
the profile opened again from the in-memory backend), which must keep
the quota, keep the order, and read back after reopening exactly what
was flushed. Another, `http_cache`: a response's fields, a request's
and a series of times fed to the cache, which must never reuse what
its freshness says is stale, checked independently, never store
`no-store`, and stay within its bounds. `dom_api` gains `localStorage`
and `sessionStorage` calls. *(corrected)* `storage` drives the
engine's own areas through `engine::StorageDriver` over a profile in
`Memory`, against a model of the areas written again in the target,
with the profile written, opened again, copied as a crash leaves it,
and damaged a byte at a time (refused, or read without a panic).
`dom_api`'s pages are `data:` ones, refused storage: its new calls are
that refusal and `StorageEvent` made, initialized and dispatched. The
`cookies` target gains the jar written as records and read back, with
records of the input's bytes among them. Seeds for sequences a fuzzer
takes long to find are written by `fuzz-seeds/seeds-m28.py`.

**The web-platform tests.** M27's testharness.js support takes
`webstorage`: its `.window.js` tests and the pages that use no frame or
second window (the `storage` event's tests open one, and are left
out). Fetch's `http-cache` tests need Python handlers and are left out
by the rule. *(corrected)* 25 tests: two of the `storage` event's,
whose harness makes its frame by `createElement("IFRAME")`, came in
past the rule's frame check, which read the name in lower case alone;
it reads any case now. 1,239 of their 1,251 subtests pass; the 12
others set a lone surrogate (G78).

**Golden cases** continue from 128: a page whose boxes are built from
what it stored in `localStorage` and `sessionStorage` and read back,
captured from the reference browser on a new profile. *(corrected)*
Served over `http:` (`<meta name="reference-load" content="http">`),
by the reference test's server and by Python's for the capture, since
a `file:` page has no storage here; the page reads its items by name,
the two browsers' orders of `key()` differing.

**Acceptance.** The roadmap's isolation tests, before and after the
profile is opened again. A page that counts its visits in
`localStorage` and in a cookie with an expiry, loaded in the real shell
twice on one profile directory, showing two the second time, and its
`sessionStorage` empty again. A benchmark, `benches/storage.rs`: 1,000
`setItem`s and `getItem`s, the flush of 1,000 changed items, a profile
opened with 3,000 cookies and 100 origins' items; and a page of 100
cacheable images and stylesheets loaded twice, the second time from
the cache. *(corrected)* The page's 100 resources are stylesheets. The
visit counter runs through the shell's window in a session test, two
windows in turn on one profile, and in the real shell, two
`--print-to-pdf` runs on one `--profile` directory, then a window
killed outright, whose visit the once-a-second write had kept. The
benchmark found `Jar::restore` quadratic, a search of the jar for each
cookie's namesake and an eviction after each: an index and one trim to
the limits by sorting took the profile's open from 25ms to 11.5ms.
Numbers in `docs/perf-budgets.md`.

## 2. Not in this milestone, said rather than hidden

- *IndexedDB*, on the same profile. M36.4.
- *The HTTP cache on disk*, revalidation (`ETag`, `Last-Modified`, the
  304 that answers them), `stale-while-revalidate`, and navigations'
  documents cached. M38.
- *The Cache API and service workers*. M39.
- *A user's view of what is stored, and clearing it*; storage
  pressure, eviction of origins, `navigator.storage`. M36.
- *Session restore*: `sessionStorage` and session cookies kept for a
  browser that is started again. M37, with tabs.
- *The `storage` event across tabs*. M37.
- *Partitioned cookies and storage* (CHIPS, storage partitioned by
  top-level site). M33.
- *A profile shared by processes*, the browser process owning it.
  M34.
- *Encryption of the profile at rest*. M33.
- *Lone surrogates in storage keys and values*. G78.

## 3. Order of work

Each step a vertical slice, with its tests, a sabotage pass and the
gate.

1. *The profile*: the `profile` crate over `redb` (D10), its tables,
   opening and refusing, writing in transactions, its unit tests.
2. *The shell and the engine*: `--profile`, `--temporary-profile`,
   the default directory, `Engine::set_profile` and
   `flush_profile`, the batched writes, the window's close.
3. *Cookies kept*: `Jar::persistent` and `restore`, the jar written
   when it changes and loaded with the profile.
4. *Storage areas*: the engine's `localStorage` areas over the
   profile and the tab's `sessionStorage` areas, the quota, the
   natives.
5. *`Storage` in the bootstrap*: the `Proxy` and its traps, the
   window's getters and their `SecurityError`, `QuotaExceededError`,
   `StorageEvent` and the `storage` event.
6. *The HTTP cache*: `Cache-Control`, `Expires`, `Age` and dates read,
   freshness, storing and lookup with `Vary`, the partition, the cache
   modes and `Request.cache`, invalidation, the bounds.
7. *Isolation* (M28.5): the tests from both sides, before and after
   the profile is opened again.
8. *Fuzzing*: the `storage` and `http_cache` targets, `dom_api`'s new
   calls.
9. *The web-platform tests*: `webstorage`, expectations.
10. *Golden cases* from 128.
11. *Acceptance*: the visit counter in the real shell, the benchmark,
    T4.3, fuzz runs.
12. *The documents*.

*(corrected)* Steps 2 and 3 were one, cookies being the first thing
kept, and steps 4 and 5 one, the areas having no test without their
bindings: the milestone took ten steps, the fuzzing its sixth.
