# Design note: scripting essentials (gap batch A)

The first batch of the G-list worked through by its priorities
(`silver-work/notes/gap-priorities.md`, 28 October 2026). Written before
the code, as the record of the decisions, and corrected to the code
when the batch ends: where they disagree, the code wins, and section 4
lists the places. What ships is described in `docs/architecture.md`
"Current state", and the limits it adds live in `docs/security.md`.

A page's scripts reach a document here, but not a handful of the
interfaces almost every script calls as it starts. There is no `URL`
class (G68), so `new URL(location.href)` throws, and no `blob:` URL
(G171); no `TextEncoder` or `TextDecoder` (G69); no `matchMedia()`
(G86); no `MutationObserver` (G49); no `IntersectionObserver`,
`ResizeObserver` or `requestIdleCallback()` (G65); no `CSS` namespace
(G88); no `FileReader` (G75); no `BroadcastChannel` (G84); no element
by its id as a global (G54), nor `document.scripts`, `document.links`
or `baseURI` (G105); `alert()`, `confirm()` and `prompt()` answer as
suppressed dialogs do, the user never asked (G56); and a binding's
`toString()` shows its script, so libraries that test for native
functions take ours for polyfills (G135). Google's account bar fails on
two of them as it starts (G86, G68).

This batch gives the engine all of them. Its acceptance is WPT's
directories for them (fetched for it: `url`, `encoding`,
`intersection-observer`, `resize-observer`, `requestidlecallback`,
`FileAPI/reading-data-section`, `FileAPI/FileReader`,
`webmessaging/broadcastchannel`, `dom/nodes`' MutationObserver tests,
`css/cssom`'s tests of the `CSS` namespace,
`html/webappapis/user-prompts` and named access on the window), with
expectations, and pages whose lines are the reference browser's.

The documents this note follows, at their versions of 28 October 2026:
the URL Standard (the API, the basic URL parser with its state
overrides, `URLSearchParams`); the File API (`Blob`, `FileReader`, the
blob URL store, `URL.createObjectURL`, blob URL resolution and fetching
by the Fetch Standard's scheme fetch); the Encoding Standard's API
(`TextEncoder`, `TextDecoder`, streaming); CSSOM View's `matchMedia()`
and `MediaQueryList`; DOM's mutation observers; Intersection Observer;
Resize Observer; Cooperative Scheduling of Background Tasks
(`requestIdleCallback`); CSSOM's `CSS` namespace and CSS Conditional
Rules 3's `CSS.supports()`; HTML's `BroadcastChannel`, the Window's
named properties, user prompts and the "update the rendering" steps;
Web IDL's `[LegacyUnenumerableNamedProperties]` and its named
properties object.

What the standards leave open, or say but are worth checking, was put
to the reference browser (Chrome 156.0.8078.47, headless, on this
machine) before this note was written, its page served on `127.0.0.1`
(scratchpad `ba/pages/msgs.html`). Its lines, which the bindings' words
follow:

- `new URL('nope')`: TypeError "Failed to construct 'URL': Invalid
  URL"; a bad base: "Invalid base URL"; a setter given a bad `href`:
  TypeError "Failed to set the 'href' property on 'URL': Invalid URL".
  `URL.canParse('nope')` false, `URL.parse('nope')` null.
  `URL.createObjectURL('x')`: "Overload resolution failed." A blob URL
  is `blob:` and the creating document's origin, a slash and a UUID.
  `url.searchParams` is the same object each time.
- `new TextDecoder('nope')` and `'replacement'`: RangeError "The
  encoding label provided ('nope') is invalid."; a fatal decoder's bad
  bytes: TypeError "The encoded data was not valid."; `'latin1'`'s
  encoding is `windows-1252`; `encodeInto('h' + U+00E9, 2 bytes)` reads
  1 and writes 1.
- `matchMedia('(nope')` is a `MediaQueryList` whose media is the text
  given as it serializes; a query that does not parse never matches.
- `MutationObserver.observe()` with no type: TypeError "The options
  object must set at least one of 'attributes', 'characterData', or
  'childList' to true."; with an old value of a type set false, its
  own sentence for each.
- `IntersectionObserver` defaults: a null root, `rootMargin` and
  `scrollMargin` "0px 0px 0px 0px", thresholds `[0]`, delay 0,
  `trackVisibility` false; a bad margin a SyntaxError "rootMargin must
  be specified in absolute length units or percent."; a threshold past
  1 a RangeError "Threshold values must be numbers between 0 and 1".
- `CSS.supports('display', 'flex')`, `('display: grid')` and
  `('selector(:has(a))')` true, a condition with an unknown part false;
  `CSS.escape('1a b' + U+0000 + '-')` is `\31 a\ b` U+FFFD `-`.
- `FileReader` starts EMPTY (0) with null result and error; a second
  read while one is going: InvalidStateError "The object is already
  busy reading Blobs."
- A closed `BroadcastChannel`'s `postMessage`: InvalidStateError
  "Channel is closed".
- Named access: an element with an `id` is a global (an object), a
  `form`'s and an `img`'s `name` too; `'named' in window` true, and the
  name not among `Object.getOwnPropertyNames(window)`.
- `document.scripts` is an `HTMLCollection`; `baseURI` the document's
  URL.
- A binding's `toString()`: `function createElement() { [native code]
  }`; a getter's `function get title() { [native code] }`; an interface's
  `function URL() { [native code] }`.
- `alert`, `confirm` and `prompt` have length 0.

## 1. Decisions

### URL and its setters

`URL` is a class of the bootstrap's over the engine's URL parser
(`net::url`), one native, `native.url`: a parse of an input against a
base, giving the record's parts or a failure, and a setter's run. The
setters are the standard's: the basic URL parser with a **state
override** (the protocol, username, password, host, hostname, port,
pathname, search and hash states as the standard writes them) is added
to `net::url::parser`, which was written as the standard's state machine
for this. `URL.parse()`, `URL.canParse()`, `toJSON()` and `href`'s
setter as written; `searchParams` is a `URLSearchParams` (M27's, in the
bootstrap) whose changes write the URL's query back and which the
URL's `search` setter updates, as the standard links them. The parser
is fuzzed already (`url_parse`); the setters join it.

### `blob:` URLs (D41)

**A store a process.** `URL.createObjectURL(blob)` registers the blob's
bytes and type in a **blob URL store of the process's** (one for the
engine in one process, one for each renderer), keyed by the URL, with
the creating global's storage key; `revokeObjectURL()` and the
creating global's end remove it. The URL is `blob:` and the global's
origin's serialization, `/`, and a UUID (`null` for an opaque origin).
A blob's bytes are already immutable in the bindings, so the store
holds them shared (`Arc<[u8]>`), not copied again.

**Resolved before the network.** The engine's fetcher, every request a
document or a worker makes (`fetch`, XHR, images, scripts, styles, a
worker's script), is wrapped by one that answers `blob:` URLs from the
store before a request leaves the renderer, as the Fetch Standard's
scheme fetch does: a `GET` alone, the blob's bytes with its type and
`Content-Length`, a `Range` answered with 206 as the standard now
allows; a URL of another storage key's, revoked or unknown, a network
error. A worker of a blob URL starts from it (its origin the
creator's). A `blob:` URL a frame or a tab is *navigated* to, which
must cross to the browser, stays with downloads made by script on G171.

**Bounded.** The store holds at most `MAX_BLOB_URLS` URLs a process and
their bytes are counted against `MAX_BLOB_URL_BYTES`; past either,
`createObjectURL` still answers a URL, which then resolves to a network
error, as a store that ran out would (the reference browser's own
limits are its quota's).

### `TextEncoder` and `TextDecoder`

The bindings', over `encoding`: `TextEncoder` writes UTF-8 (in the
bootstrap, which has the code points), `encodeInto` as the standard
counts; `TextDecoder` holds a decoder of the crate's (a streaming
`Decoder`, by a number the realm owns) for `stream: true`, its label
and options read as the standard says (`replacement`, and the labels
that map to it, are invalid here), `fatal` a TypeError, `ignoreBOM`.
The streaming forms (`TextEncoderStream`, `TextDecoderStream`) wait for
Streams (G67).

### `FileReader`

The bindings', over `Blob`'s bytes: `readAsArrayBuffer`,
`readAsBinaryString`, `readAsText` (through `TextDecoder`'s decoders,
the blob's type's charset or the one given), `readAsDataURL`, `abort`;
the standard's states and its events (`loadstart`, `progress`, `load`
or `error` or `abort`, `loadend`) each queued as a task, the read itself
taken at once as the bytes are already in the realm.

### `matchMedia()` and `MediaQueryList`

`matchMedia()` parses its query with the style crate's media query
parser and answers a `MediaQueryList` whose `media` is the query's
serialization and whose `matches` is the query evaluated now against
the engine's device (the viewport, the preferences, the resolution).
The engine's "evaluate media queries and report changes" step runs in
each frame's update before the animation frame callbacks: every live
list with listeners is evaluated again, and a `change` event fired at
each whose answer changed, in the order they were made. A list without
listeners is not kept (nothing could see its changes).

### `requestIdleCallback()`

An idle period starts when a frame's update is done and no task is
queued, and ends at the next frame's deadline, or after 50 ms, or at
the next timer; its callbacks run as tasks in it, each given an
`IdleDeadline` whose `timeRemaining()` counts down to the period's end.
A callback with a `timeout` that passes is queued as an ordinary task,
`didTimeout` true. `cancelIdleCallback()` removes one.

### Mutation observers (D42)

**Records from the DOM, grouped by its operations.** The `dom` crate's
document gains an *observed changes* queue, kept only while some
observer is registered: each change as DOM's "queue a mutation record"
needs it, a child list's added and removed nodes with their siblings,
an attribute's name, namespace and old value, a text's old data. The
engine opens and closes an *operation* around each binding's call
(`appendChild`, `innerHTML`, `replaceChildren`, a `DocumentFragment`'s
insertion), so that one call's changes are one record as the standard
groups them; the parser's insertions, outside any, are a record each.

**Registrations in the engine.** An observer's registrations (its
node, its options, its transient registrations on removed subtrees) are
kept by the engine for the document, so that a change is matched
against the inclusive ancestors of its target without the realm. The
records go to their observers' queues, and the realm's mutation
observer microtask (DOM's "notify mutation observers") takes them,
the records made objects there and each callback called with them, in
the order the observers were made. A queue holds at most
`MAX_MUTATION_RECORDS`; past it, its oldest go.

### Resize and intersection observers (D45)

**In the frame, after layout.** Both run in the frame's update as HTML
orders it: after layout, the resize observations are gathered,
broadcast and gathered again at greater depth, up to the standard's
limit, an error event for those skipped; then the intersection
observations are computed for every observer. Their geometry comes
from the frame's layout in one walk of the paint order for the
observed elements (the walk `Engine::border_box` makes for one), so
that a hundred observed elements cost one walk.

**What they measure.** A resize observation reports the content box,
border box and device pixel content box sizes, in the element's
writing mode's inline and block sizes. An intersection is the target's
border box clipped by its ancestors' clips and the root's box (the
viewport, or an element root's padding box), with `rootMargin` and
`scrollMargin` applied as the standard says, against each threshold.
In a frame the implicit root is the frame's own viewport, not the
top-level one (section 2).

### `BroadcastChannel` (D46)

A message goes to every other `BroadcastChannel` of the same name and
storage key in the browser, in documents and dedicated workers alike,
as a structured clone (M31's), queued as a task on each receiver in the
order sent. In one process the engine's context carries them. Between
renderers the browser does, as it relays a storage area's changes
(M28, M37): a renderer sends `ToBrowser::Broadcast` with the storage
key the browser holds it to (a renderer naming another site's is
killed), and the browser sends it to each other renderer of the context
that hosts that key, which hands it to its channels. A closed channel
sends and hears nothing. Shared and service workers' channels wait for
them (G82; a service worker's global has none yet).

### Named properties of the window (D44)

The window's named properties object (Web IDL 3.7.4): a `Proxy` set
between `Window.prototype` and `EventTarget.prototype`, whose traps
answer a name with the document's elements of that `id`, or of that
`name` among `embed`, `form`, `img` and `object` (and a frame's
`WindowProxy` by its browsing context's name), the first in tree order,
or an `HTMLCollection` when there are several. The global's own and
`Window.prototype`'s properties come first, as lookup reaches them
before the proxy; the names are not enumerable and not own properties
of the window (`[LegacyUnenumerableNamedProperties]`). The engine
answers the lookup from the document's index of ids, which
`getElementById` uses, and its name index.

With them, G105's members: `document.scripts`, `document.links`,
`Node.baseURI`, `getComputedStyle()` read by a hyphenated name, and an
`img` made by `new Image()` fetching its `src` as one made by
`createElement` does.

### User prompts (D43)

`alert()`, `confirm()` and `prompt()` ask the user. The page's script
waits for the answer, as the standard has it (its event loop paused).
The browser shows the prompt over the tab's content, drawn by `ui`:
the origin that asks, the message (trimmed of control characters,
at most `MAX_PROMPT_CHARS`), OK and, for `confirm` and `prompt`,
Cancel, and `prompt`'s text field, keyboard first. In one process the
engine's host asks the shell, which runs the dialog's own loop; in
renderers the renderer sends `ToBrowser::Prompt` and blocks on
`ToRenderer::PromptAnswered` (its other frames wait with it, as one
agent's do). A frame sandboxed without `allow-modals`, a document being
unloaded, and a tab not shown answer at once as now (no dialog: `alert`
nothing, `confirm` false, `prompt` null). Tests and the headless shell
answer by their host.

### A binding's `toString()` (D47)

`Function.prototype.toString` is the bindings' own: a function the
bindings made (a method, an accessor, an interface's constructor; the
WebGPU and WebGL bindings too, at their first use) answers `function
NAME() { [native code] }`, `get NAME` and `set NAME` for accessors, from
a `WeakSet` the bindings fill as they expose each interface; any other
function as QuickJS writes it. `Function.prototype.toString` is in the
set itself.

### Bounds

`MAX_BLOB_URLS` (10,000 a process) and `MAX_BLOB_URL_BYTES` (the
process's blob URLs' bytes, 512 MiB), `MAX_MUTATION_RECORDS` (an
observer's queue, 100,000), `MAX_PROMPT_CHARS` (a prompt's message and
default, 10,000 characters), and a `BroadcastChannel` message within
`ipc::MAX_MESSAGE` (past it, a `DataCloneError` at the sender).

### Fuzzing

`url_parse` gains the setters: any input, base and setter value, every
URL written back parsing to itself. `mutation` registers observers on
the documents it changes and holds their records to a model of the
changes made. `encoding_decode` decodes in pieces as `stream: true`
would, the pieces' text the whole's.

## 2. Not in this batch, said rather than hidden

- *A `blob:` URL navigated to* (a frame's or a tab's), and downloads
  made by script: G171.
- *Streams' forms*: `TextEncoderStream`, `TextDecoderStream`,
  `Blob.stream()` (G67).
- *An intersection with the top-level viewport* for a target in a frame
  (the frame's own viewport is the implicit root here), and
  `trackVisibility`'s occlusion test (`isVisible` always false).
- *Prompts' "don't show more"* and the print and beforeunload dialogs
  (G57).
- *Shared and service workers' channels* (G82).

## 3. Order of work

1. *URL*: the parser's state override, the `URL` class, `URL.parse`,
   `canParse`, `searchParams` linked; `blob:` URLs and their store,
   resolved before the network (D41).
2. *Text and files*: `TextEncoder`, `TextDecoder`, `FileReader`, the
   `CSS` namespace, the bindings' `toString()` (D47).
3. *`matchMedia()` and `requestIdleCallback()`*.
4. *Mutation observers* (D42).
5. *Resize and intersection observers* (D45).
6. *`BroadcastChannel`* (D46).
7. *Named access and G105's members* (D44).
8. *User prompts* (D43).
9. *The acceptance*: WPT's directories, pages against the reference
   browser's lines, T4.3, fuzzing, documents.

## 4. Corrected to the code

Filled in at the last step.
