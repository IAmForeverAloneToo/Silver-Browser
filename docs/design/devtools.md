# Design note: DevTools v0 and the remaining storage

Roadmap M36. Written before the code, as the record of the decisions,
and corrected to the code when the milestone ends: where they
disagree, the code wins, and section 4 lists the places, marked
*(corrected)*. What ships is described in `docs/architecture.md`
"Current state", and the limits it adds live in `docs/security.md`.

The browser has no tools of its own for looking at a page. Its
console is a line on the shell's standard error, `console: <level>
<text>` (M24): a `console.log` of an object prints `[object Object]`,
because every argument is turned into a string by ECMAScript's
`ToString` inside the realm; an uncaught exception keeps its first
stack frame and loses the rest; an entry carries no time, no source
and no position once it is text. Nothing records a request: the HTTP
client keeps no timing, the headers it adds and the redirects it
follows are seen by no one, and a failed image or stylesheet says
nothing anywhere. No API says which rules matched an element: the
cascade's `Matched` and `Applied` are private and drop the rule, the
sheet and the selector before they sort. Nothing shows what a site
keeps, and IndexedDB, which pages keep their data in and which the
web-platform tests' interface lists expect, does not exist. Since M34
a page's DOM, styles and console live in a sandboxed renderer, and the
only message about them that reaches the browser is its console text.

This milestone gives the browser DevTools of its own, built of the
engine itself (D9, D11): a panel docked under the page, an HTML page of
the browser's own drawn by the engine, with four tools (a console with
objects and errors, an element inspector with the rules that matched
and the computed values, a network log, a storage view), and the
storage it lacks: IndexedDB on the profile, a view of what each site
keeps, and its clearing.

The documents this note follows, at the versions current when it was
written (20 October 2026):

- the Console Standard (WHATWG, Living Standard): the methods, the
  formatter (`%s`, `%o`, `%c`), `assert`, `count`, `time`, `group`,
  `table`, `trace`, and what "a printer" may do with an object;
- Indexed Database API 3.0 (W3C): the key and key range types and
  their order, key paths and their evaluation, the key generator, the
  object store and index operations, the request and transaction
  lifecycles (`upgradeneeded`, `blocked`, `versionchange`,
  auto-commit, `abort` and the errors), and the cursor;
- the Storage Standard (WHATWG): storage buckets, usage and quota,
  `navigator.storage.estimate()` and `persisted()`, and clearing a
  site;
- the HTML Standard's structured serialize (section 2.7), which
  IndexedDB values pass through;
- the Chrome DevTools Protocol, declined below (D11).

## 1. Decisions

### The protocol (D11)

**Our own, small, JSON (D11 as written: custom first).** The Chrome
DevTools Protocol is some forty domains and a thousand methods and
events, each shaped by Chromium's internals (frame trees, CSS style
sheet headers, the V8 runtime's remote objects); speaking it would be
the bulk of the milestone for tools that are not wanted (D11's
condition, "CDP-compat only if external tools wanted", does not hold).
The protocol here is what the four panels need: a request is a JSON
object `{"id": n, "method": "domain.verb", "params": {...}}`, an answer
`{"id": n, "result": ...}` or `{"id": n, "error": "..."}`, and an event
`{"event": "domain.what", "params": {...}}`.

**A crate of its own, `devtools`**, a leaf below `engine`, with
`#![forbid(unsafe_code)]`: a JSON value, its parser and writer (no
crate the workspace does not have: `serde_json` is a test dependency
alone, and a parser of a few hundred lines is fuzzed below), the
protocol's messages as Rust types, the console's and the network log's
records, and the frontend's files (HTML, CSS, script) included in the
executable. Every message is bounded (`MAX_MESSAGE_BYTES`, 16 MiB, and a
nesting depth, `MAX_JSON_DEPTH`, 64) and the parser refuses what is
past either.

### Where the panel is, and what runs it

**A panel docked under the page**, toggled by F12 and by Ctrl+Shift+I
(Cmd+Option+I on macOS), which the shell does not take today; it
takes the window's lower part, `PANEL_SHARE`, 40% of the height under
the toolbar, the page above it laid out to what is left. The session
gets a second view and regions: every place that tests `y < TOP` today
(the pointer, the wheel, presses and releases, the cursor's shape, the
input method's area, the scrollbar's drag) asks which region a point
is in, and gives the view its own coordinates; the keyboard goes to the
view last clicked, as a press decides focus between the field and the
page today. The status bubble moves to the page's own bottom.

**The frontend is a page of the browser's own: `devtools:`**, HTML, CSS
and script bundled in `devtools`, drawn and scripted by the engine
like any page, so that its trees, tables and scrolling are the
engine's (D9, "the engine is the toolkit"). *In the many-process build
it runs in a renderer of its own*, under a lock of its own,
`Lock::DevTools`, sandboxed like any renderer, granted no network and
no storage: what it shows is the inspected page's DOM, its rules, its
console's text and its responses' headers and bodies, all of them the
page's to choose, and a bug of the engine's that such a string reaches
is then a bug in a sandboxed process that holds nothing but the
panel. *In the one-process build* it is an engine of the browser's own
process, as everything is there.

**Its bridge is two messages and one global.** The `devtools:` document
alone, made by the browser (a page no navigation can reach: the
scheme is refused to the address bar, to links and to scripts), gets a
global `devtools` with `send(message)` and an `onmessage` handler;
`send` queues a JSON string for the browser, and the browser's
messages arrive as tasks that call `onmessage`. Between processes they
are `ToBrowser::DevTools { view, message }` and `ToRenderer::DevTools
{ view, message }`; in one process the same strings pass by method. The
`View` trait gains `devtools_send`, `take_devtools` (the panel's side)
and `inspect`, `take_inspected` (the inspected page's side), defaulting
to nothing.

**The router is the browser's**, in the shell beside the session: it
takes the frontend's requests, answers those about the browser's own
records (the network log, the storage of every site, the console's
history) itself, and passes those about the page (its DOM, its styles,
an expression to evaluate, a node to highlight) to the inspected view
with `inspect`, whose answer comes back through `take_inspected`. A page
navigated keeps its panel; the panel is told, and its trees start over.

### The console (M36.1)

**An entry is a record, not a line.** `ConsoleEntry` gains its source
(the console API, an uncaught exception or rejection, the network, a
policy such as CSP, a worker, the engine), its time on the document's
clock, its position when it has one (URL, line, column), a stack when it
is an error, and its arguments as *previews*: each argument a snapshot
taken when it was logged, a primitive as itself, an object as its class
and up to `PREVIEW_PROPERTIES` (100) own properties to `PREVIEW_DEPTH`
(3) levels, a function as its name, an `Error` as its name, message and
stack, a node as its tag, id and classes. The text stays: the shell
still prints it to its standard error. The reference browser shows
objects live, expanded as they are when clicked; a renderer's object
cannot be held by the panel across a process, and a snapshot is what is
logged, which the panel says.

**The methods the Console Standard has** besides the five there are:
`assert`, `count` and `countReset`, `time`, `timeLog` and `timeEnd`,
`group`, `groupCollapsed` and `groupEnd`, `table`, `trace`, `dir`,
`dirxml` and `clear`, each with its standard's output.

**Errors keep their stacks.** `ScriptError` gains the stack QuickJS-ng
gives, its frames parsed (function, URL, line, column) and kept to
`MAX_STACK_FRAMES` (64); the first frame stays its position. A network
failure the engine logs (a script, an image, a stylesheet, a fetch)
is an entry of the network source with its URL, and the images and
stylesheets that fail say so now, as the reference browser does.

**The console evaluates**: an expression typed into the panel runs in
the inspected document's realm as a classic script of its own (the
page's policy aside, as `javascript:` URLs run, M24) and its value is
shown as a preview; `$0` is the node selected in the element
inspector.

### The element inspector (M36.2)

**The tree, live.** The inspected document is sent as a tree of nodes
named by their `NodeId` bits (D16), each with its tag, attributes or
text, to a depth the panel asks for and opened further as the user
opens it; a change to the document (the engine's journal, `Change`)
sends the panel the changed nodes' parents again, so the tree follows
the page without a whole snapshot each frame. The node selected is
highlighted in the page, its margin, border, padding and content boxes
drawn over the frame, and "pick" mode selects the node under the
pointer.

**The rules that matched**, through a public `matched_rules(element)`
on the style crate's `Recascader`: for each declaration block that
applied, its origin (the user agent's sheet, an author sheet with the
sheet's URL or its `<style>` element, the `style` attribute, a
presentational hint), its selector as written, its specificity, its
`@media` and `@layer`, and its declarations, each marked overridden when
a later one in the cascade's order set the same longhand. The
cascade's collection keeps, for this, the rule's and the sheet's
indices that it drops today; selector text is the rule's prelude
written back by the serializer's token writer. The *computed* values
are `serialize::computed_value` for the properties it knows, as
`getComputedStyle` gives them.

**Edits.** An attribute is added, changed or removed, and an element's
`style` declarations toggled or edited, through `Engine::mutate`, as a
script's change would be (the `style` attribute's CSSOM bookkeeping
included, so that a page's CSP does not refuse an edit the user made).

### The network log (M36.3)

**The client says what it does.** `net::Client` takes an observer, and
tells it each request as sent (its final headers, the ones `net` adds
included: `User-Agent`, `Accept`, `Referer`, `Origin`, `Cookie`), each
redirect followed, a response's head (status, headers, whether from the
HTTP cache), its body's bytes as they arrive, and its end or failure,
each with the time. One client serves a tab in either build (the
engine's `Network` in one process, the tab's `NetworkService` in the
browser in many), so the observer is the tab's log, whatever asked:
documents, frames, sheets, scripts, images, `fetch()`, XHR (synchronous
too), workers and `importScripts`.

**The log** keeps `MAX_NETWORK_ENTRIES` (1,000) requests, the oldest
dropped, each with its destination, initiator, timings (start, the
connection made, the head, the end), sizes, and the first
`MAX_BODY_PREVIEW` (1 MiB) of its body; a body the renderer was not
given (Opaque Response Blocking, M34) is not shown in the panel either,
which shows what the page could read. The panel lists them, filters by
type, and shows a request's headers, timing and body (text as text, an
image as the image).

### IndexedDB (M36.4)

**On the profile (D10).** Three new tables in the profile's database,
`SCHEMA` 3, each value sealed as the others are (AES-256-GCM, M28):
`idb_databases` (a storage key and a database's name, to its version,
its stores and their indices, key paths and key generators),
`idb_records` (a storage key, a database, a store and an encoded key,
to the value's structured serialization), and `idb_indices` (a storage
key, a database, a store, an index, an encoded index key and a primary
key, to nothing). Keys are encoded so that their bytes sort as
IndexedDB orders keys (numbers, then dates, then strings, then binary,
then arrays, each array element by element), so a range of keys is a
range of the table and `getAll` a scan.

**Where a transaction runs.** The data lives with the profile, in the
browser in the many-process build: a renderer's script evaluates key
paths and serializes values (they are its objects), and sends each
request (put this record with these index keys, get this range, delete,
count, open a cursor's next step) to the browser's `StorageService`,
which keeps the transaction's writes in memory, answers reads from them
and the database together, and commits them in one redb transaction
when the transaction completes; an abort drops them. Transactions whose
scopes overlap with a writer among them run one after another, by a
lock the service keeps per database, in the order they were made. The
calls are the synchronous `Link::call` the storage already uses; the
script sees each result as a task, as the specification's requests
complete asynchronously. In one process the same service runs in the
engine.

**The API**: `indexedDB` (`IDBFactory`: `open`, `deleteDatabase`,
`databases`, `cmp`), `IDBOpenDBRequest` with `upgradeneeded` and
`blocked`, `IDBDatabase` (`createObjectStore`, `deleteObjectStore`,
`transaction`, `close`, `versionchange`), `IDBTransaction`
(`objectStore`, `abort`, `commit`, `complete`, `error`, `abort` events,
auto-commit when no request is pending at the end of a task),
`IDBObjectStore` (`put`, `add`, `get`, `getKey`, `getAll`, `getAllKeys`,
`delete`, `clear`, `count`, `createIndex`, `deleteIndex`, `index`,
`openCursor`, `openKeyCursor`), `IDBIndex` (the reads and the cursors,
`unique` and `multiEntry`), `IDBKeyRange`, `IDBRequest`, `IDBCursor` and
`IDBCursorWithValue` (`continue`, `advance`, `continuePrimaryKey`,
`update`, `delete`), in windows and workers. Its events dispatch with a
microtask checkpoint after each listener, as the specification's "fire a
success event" asks, through the bootstrap's dispatch generator.

**Its limits**: `MAX_IDB_VALUE_BYTES` (64 MiB) a value,
`MAX_IDB_KEY_BYTES` (64 KiB) an encoded key, and `IDB_ORIGIN_QUOTA`
(512 MiB) a storage key's databases together, past which a write fails
with `QuotaExceededError`.

### The remaining storage

**What a site keeps, and clearing it.** The profile gains a listing of
the storage keys it holds anything for, with each one's usage (its
`localStorage` items, its IndexedDB databases, its cookies), and a
clearing of one; the storage panel shows the inspected site's
`localStorage`, `sessionStorage`, cookies and IndexedDB databases (their
stores and records), with an item's deletion and the site's clearing.
`navigator.storage.estimate()` gives a storage key's usage and quota,
and `persisted()` false (a profile does not evict yet, below).

### Bounds

Every constant above is a bound a page cannot pass: `MAX_MESSAGE_BYTES`
and `MAX_JSON_DEPTH` the protocol's, `PREVIEW_PROPERTIES` and
`PREVIEW_DEPTH` an argument's preview, `MAX_STACK_FRAMES` a stack,
`MAX_NETWORK_ENTRIES` and `MAX_BODY_PREVIEW` the log,
`MAX_IDB_VALUE_BYTES`, `MAX_IDB_KEY_BYTES` and `IDB_ORIGIN_QUOTA` the
database; the console keeps its `MAX_CONSOLE_ENTRIES` and
`MAX_CONSOLE_MESSAGE`. The panel and its renderer exist only while the
user has it open.

### Fuzzing

`devtools_json` (the parser and writer, every parsed value written back
the same), `idb_keys` (a key encoded and decoded the same, and the
bytes of two keys ordered as the keys are), and `idb_ops` (a script of
IndexedDB operations against the store and against a model of it in
memory, compared after each).

## 2. Not in this milestone, said rather than hidden

- *The sources panel and a debugger* (breakpoints, stepping, scopes,
  source maps; T4's fourth panel): QuickJS-ng's debugger interface is
  not exposed by `rquickjs`. G145.
- *The performance panel* (T4's fifth): the frame statistics M13 keeps
  are not recorded over time. G146.
- *Live objects in the console*: a preview is a snapshot (above).
- *The Chrome DevTools Protocol and remote debugging* (D11).
- *The panel undocked into a window of its own, resized by a drag, or
  docked at the side*: the window is one surface until M37's tabs and
  windows. G147.
- *The panel's own preferences kept* (its height, the tool open): with
  M37's settings.
- *The storage of other kinds the panel could show*: the HTTP cache
  (M38), the Cache API and service workers (M39).
- *Eviction of a site's storage under pressure, and persistent
  storage* (`navigator.storage.persist()`): the profile does not evict
  yet. G148.
- *IndexedDB's `getAllRecords` and the draft's other additions beyond
  3.0*, and `IDBObjectStore.prototype.getAll` with a direction (the
  draft's 4.0).

## 3. Order of work

Each step a vertical slice, with its tests, a sabotage pass and the
gate.

1. *The `devtools` crate*: the JSON value, parser and writer, the
   protocol's types, the `devtools_json` fuzz target.
2. *The panel*: regions and a second view in the session, F12 and
   Ctrl+Shift+I, the `devtools:` page (its frontend shell, tabs and
   styles) in a renderer of its own or the browser's engine, the
   bridge (the global, the IPC messages, the `View` methods) and the
   router.
3. *The console*: entries as records, previews, stacks, the Console
   Standard's methods, network and policy sources, the console panel
   with filters, groups and evaluation.
4. *The element inspector*: `matched_rules`, the tree and its updates,
   the styles and computed panes, edits, the highlight and the pick.
5. *The network log*: the client's observer, the log, the network
   panel.
6. *IndexedDB's store*: keys and their encoding, key ranges, the
   profile's tables, the service's transactions, the `idb_keys` and
   `idb_ops` fuzz targets.
7. *IndexedDB's API*: the bindings in windows and workers, the
   requests' and transactions' lifecycles, cursors, the IPC.
8. *The remaining storage*: usage, the listing and the clearing, the
   storage panel, `navigator.storage`.
9. *Acceptance*: the to-do list made a demo with its entries kept in
   IndexedDB and a bug of its own, debugged through the panel alone in
   a session test: the error in the console, the element inspected and
   its rule edited, the request in the network log, the records in the
   storage panel (M36.5).
10. *T4.3 and the fuzz runs*.
11. *The documents*.

## 4. Corrected to the code

Where the milestone's code came out otherwise than the sections above
planned, *(corrected)*, by section. The step notes say how each was
found.

*The protocol.* *(corrected)* `MAX_MESSAGE_BYTES` is 8 MiB, not 16:
half of `engine::ipc::MAX_MESSAGE`, so that a message always fits in
one between processes, which a constant assertion in `engine::ipc`
holds. The parser reads a lone escaped surrogate as U+FFFD and a
number past a double's range as the largest double (step 1).

*Where the panel is, and what runs it.* *(corrected)* The bridge is
four messages, not two: the panel's page sends and hears
`ToBrowser::DevTools` and `ToRenderer::DevTools`, the page inspected
hears `ToRenderer::Inspect` and answers with `ToBrowser::Inspected`.
One kind each way would let a page's renderer answer as the panel; the
router reads the panel's view for requests and the inspected view for
answers, never the other way, and a page's tab that hears
`ToBrowser::DevTools` kills its renderer. The panel's page is not
navigated to: `Engine::show_devtools_panel` makes it from a response
of its own (CSP `default-src 'none'`, its inline script and style
alone) through the streamed document path, and the renderer calls it
for a `Navigate` to `devtools:panel` from the browser; a navigation of
a page's to `devtools:panel` is a network error, as any unknown
scheme's. A page has the bridge only when its engine was made a
panel's and it is under the panel's URL. In one process the panel is
an engine drawing in software beside the page's GPU backend.
`Lock::DevTools` is refused every fetch by the network service, and its
opaque origin keeps it from storage. The router is generic over a
trait of the five methods it uses, so that its rules are tested
without a view: it answers `browser.*` itself (`browser.version` in
v0), passes the rest to the page, and answers the requests still
waiting "the page navigated" when the page's URL changes. The panel
takes the keyboard when it opens and when pressed in, the page when it
is; the status bubble is drawn at the page's bottom, above the panel
(step 2).

*The console.* *(corrected)* The console namespace is the bindings'
own script now, every Console Standard method, with one native left,
`consoleMessage`, that formats the text as before. Previews are made
in that script as JSON text, by brand checks and own-property
descriptors, so that no getter of the page's runs; a `Proxy`'s traps
still can (G160). Besides `PREVIEW_PROPERTIES` and `PREVIEW_DEPTH`,
`PREVIEW_NODES` (1,000) and `PREVIEW_STRING` (10,000) bound one call's
previews, and past `MAX_PREVIEW_BYTES` (64 KiB) a call has its text
alone. QuickJS-ng's `stack` is an accessor of `Error.prototype`'s,
whose original getter the script keeps: a call's place is its stack's
first frame past the bindings' own, and the stack is kept for an
error, an assertion and a trace alone. A call written over several
lines is placed at its last, where the reference browser places it at
its first (G152). An entry's time is when the engine took it from the
document. A policy's refusal is not logged again as a failed load; its
own line says why. Evaluation needs the page's scripts on (with them
off, the default, it answers so) and runs as an indirect `eval` in the
global scope with the original `eval`. The router keeps the console's
history (`MAX_CONSOLE_ENTRIES`), the panel open or not, cleared when
the page's URL changes. A console call in a `catch` whose heap is still
spent may itself run out of memory, where M24's native console
allocated nothing: the logger then falls back to its text (G153;
steps 3 and 9).

*The element inspector.* *(corrected)* `Recascader::matched_rules`
matches the element again on a path of its own, the cascade keeping
nothing more per declaration; a parsed rule keeps its rule's index, by
which `RuleSet::described` gives its sheet, its selectors written back
and the `@media`, `@supports` and `@layer` around it. Blocks are
ordered by their best cascade key, and a declaration is overridden
when every longhand it sets is set by a later one; an invalid
declaration still counts as setting its longhands (G154). The tree's
updates are coarse: once the panel has read the tree, a change of the
document's version is told once as `dom.updated`, and the panel reads
again what it has open (G161). The tree shows elements and text alone,
the DOM having no comments or doctypes, white space between elements
left out. The highlight is drawn over the frame the engine returns,
from the border box and the computed margins, borders and padding,
percentages resolved against the border box's width and auto margins
taken as 0 (G155). Edits are of attributes and the `style` attribute;
a sheet's rules are shown, not edited. The pick swallows the press
and its release. A node in the console, clicked, is revealed in the
element inspector by its ancestors (`dom.path`, step 9). The computed
pane lacks what `getComputedStyle` lacks, `text-decoration` among them
(G159; steps 4 and 9).

*The network log.* *(corrected)* `net::observe::Observer`, given to a
client by `with_observer`, is told each hop's request with its final
fields, each redirect, the head (from the cache or not), the body as it
comes and the end; a CORS preflight is not told. The log,
`engine::network_log::NetworkLog`, is shared by a document's frames,
and in many processes the tab's network service's client has it, so
that a renderer's document is in it. An opaque response's body is
hidden, whatever Opaque Response Blocking did with it. Times are
milliseconds since the log began. The log is not cleared when the page
navigates (the reference browser's "preserve log"), as the new
document's request comes before the router knows of the navigation; the
panel's Clear clears it (G156; step 5).

*IndexedDB.* *(corrected)* Two profile tables, not three:
`idb_records` keeps each record with its index keys (the script
evaluated them; the browser cannot), so the indices are made again in
memory as a storage key's databases are read, and `idb_databases`
keeps each database's description. A storage key's databases are held
in memory once read, ranges answered from ordered maps of encoded keys,
and written to the profile from committed transactions at the
storage's flush, in one redb transaction with the rest; an abort is
undone in memory from a journal. Keys are encoded with `0x00` escaped
and `0x00 0x00` ending a string, a binary or an array part, dates and
numbers as doubles made to sort, `-0` as 0, `MAX_KEY_DEPTH` (64) arrays
deep (step 6). IndexedDB is the window's alone: a worker's global has
no `indexedDB` (G149). Requests cross as JSON through the storage
areas' trait (`StorageOp::Idb` to the browser's storage service in
many processes, which checks the storage key as it does Web
Storage's); keys cross tagged, a string key's lone surrogates becoming
U+FFFD on the way (G151). The transactions' order is kept by the
bindings, not by a lock of the service's: those of a database in one
realm begin in the order they were made, and one whose scope an
unfinished writer's shares waits for it; transactions of two realms
(frames of one origin) are not ordered, `versionchange` is told to the
connections of the same realm alone, and `blocked` is fired but the
upgrade goes on rather than wait (G150). Each completion is a task of
its own, its event dispatched with a microtask checkpoint after each
listener, through request, transaction and database (step 7).

*The remaining storage.* *(corrected)* The storage panel's requests
are the browser's to answer, from the tab's storage and jar, for the
page shown's origin (an opaque one keeps nothing); the router never
asks the page's renderer. The panel lists `localStorage`,
`sessionStorage`, the cookies that go to the page's host, `HttpOnly`
ones too, the IndexedDB databases with each store's count and first
`MAX_PANEL_RECORDS` (100) records, their values as the structured
clone's wire text (G157), and every storage key held with its bytes.
`navigator.storage.estimate()` counts `localStorage` at two bytes a
code unit with the IndexedDB keys and values, against a quota of
`SITE_QUOTA`; `persisted()` and `persist()` give false. It is defined
on every window's `navigator`, not only a secure context's (G158), and
a worker has none (step 8).

*The acceptance* (M36.5). The to-do list is a demo of its own,
`crates/engine/tests/demos/todo-idb.html`, its entries kept in
IndexedDB and seeded from the server, with two bugs: a misspelt
function thrown when an entry is completed, and a misspelt class in the
rule that would strike a done entry out. The session test
`the_to_do_list_is_debugged_through_the_panel` drives the panel's own
controls in the one-process session: the error in the console with its
place, the entry's label revealed from the console and its rules read,
its style edited, the seed's request in the network log, the records in
the storage panel, and the list read back from the profile on a second
load. Each tool's many-process path is the tab tests' (step 9).

*T4.3.* *(corrected)* The console as the bindings' script cost a
thousand calls 29.4ms against M24's 1.06: every object's preview made
brand checks that throw, each building a stack, and the call's own
stack was built to the page's `Error.stackTraceLimit`, which a page's
`prepareStackTrace` could also answer. A call now sets both through
their original accessors (no stack while previews are made, 8 frames
for its place, 64 for an error or a trace) and puts the page's back
after: 17.6ms, an exception (G162). A realm made, 5.55 against 6.42ms,
is the bindings' new script in every realm, within its budget (G163;
`docs/perf-budgets.md`, "DevTools and IndexedDB (M36)"; step 10).
