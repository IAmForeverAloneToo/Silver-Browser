# Design note: scripting

Roadmap M24. Written before the code, as the record of the decisions,
and corrected to the code when the milestone ended: where they
disagreed, the code won, and the places are marked *(corrected)*. What
ships is described in `docs/architecture.md` "Current state".

No page runs a script. The tokenizer reads a `<script>` element's
content as raw text and the tree keeps it, and `script` is `display:
none`; nothing else looks at it. A `javascript:` link does nothing. The
document is parsed once its body has arrived whole, by a tokenizer that
reads a complete string and a tree builder that attaches an element to
its parent only when the element closes, so nothing could pause the
parser at a script, nor show a script the tree as it stands at that
point. `noscript` content is rendered, which is right while there are
no scripts, and the `scripting` media feature is `none`. There is no
Content Security Policy anywhere. This milestone adds a JavaScript
engine behind a trait of the engine's own, a narrow boundary between
scripts and the document, limits that a hostile script cannot get past,
the parser rewritten to run as bytes arrive and to pause at scripts,
the `script` element's processing model for classic scripts, and the
`script-src` part of CSP.

The standards are read at the versions current when this note was
written:

- the HTML Standard (Living Standard of 7 October 2026): sections
  4.12.1 for the `script` and `noscript` elements, 8.1 for scripts,
  realms, error reporting and the microtask checkpoint, 13.2 for
  parsing, 3.1.6 for render blocking, 7.4 for `javascript:` URLs, and
  3.1.7 for `document.title`;
- ECMAScript 2026 (ECMA-262, 17th edition, June 2026), which the engine
  implements;
- the Console Standard (Living Standard) for `console`'s formatter;
- Content Security Policy Level 3 (Editor's Draft of 16 September
  2026);
- MIME Sniffing (Living Standard of 17 July 2026) for JavaScript MIME
  types, and Fetch (Living Standard of 6 October 2026) for script
  requests, `nosniff` and the MIME types a script response may not
  have.

The texts are kept in `silver-handover/specs` beside the repository.

What the standards leave to rendering, or state but are worth checking,
was put to the reference browser (Chrome 154.0.8037.98, headless) before
this note was written:

- reading `document.title` in a script in the head before any `title`
  element gives the empty string; setting it to `"  a \n b  "` makes a
  `title` element, last in the head, and reads back `"a b"`; a
  `<title>` parsed after that is a second title element, and the first
  stays the document's title;
- an exception thrown by one script does not stop the next;
- a promise's reaction queued by a script runs after that script and
  before the next one: a microtask checkpoint follows every script;
- `(scripting: enabled)` and `(scripting)` match while scripts run;
- with scripts on, a `noscript` element's content is one text node, its
  markup unparsed, and the element draws nothing though its computed
  `display` stays `inline`: it makes no box;
- a `script` element made by a script is `async`, and one appended to
  the document runs at once, before the call that appended it returns.

What the two candidate engines were measured to do, on this machine
(release builds):

- QuickJS-ng 0.16.2 through `rquickjs` 0.14.0 builds with the MSVC
  toolchain the workspace already uses for `aws-lc-sys`, from
  bindings shipped for every target in `deny.toml`, so no `bindgen` or
  `libclang`. A realm (a runtime and a context) is made in 0.13 to 0.4
  ms and holds 96 KB. Its interrupt handler is called 2 to 5 times a
  millisecond of running code, in loops, in calls and inside native
  functions that call back into script (`Array.prototype.sort` with a
  comparator, `map`, `reduce`); returning `true` ends the script with an
  uncatchable error 0.2 to 0.5 ms later. Its heap limit makes an
  allocation past it throw a catchable `InternalError: out of memory`;
  a script that catches it and lets go of what it held goes on, the
  heap back to 99.5 KB, and the process is unharmed. Its stack limit
  throws a catchable `RangeError`, as browsers do, but its default is 1
  MiB, the whole of the main thread's stack on Windows: left at the
  default, deep recursion overflowed the process's stack and ended it.
  It has the recent built-ins (iterator helpers, `Array.fromAsync`,
  `Promise.withResolvers`, `Object.groupBy`, the set methods,
  `RegExp.escape`) and no `Intl` or `Temporal`. A loop of 2 x 10^7
  additions takes 0.78 s.
- Boa 0.22.0 (95.6% of Test262 by its own count, pure Rust) limits
  loop iterations per loop, recursion depth and stack size, and yields
  to an executor every so many instructions only in its asynchronous
  top-level evaluation, not inside a call a native function makes. It
  has no heap limit: the process's allocator fails, and a failed
  allocation aborts the process.

## 1. Decisions

**The engine: QuickJS-ng (D1).** The roadmap's working assumption is
taken: QuickJS-ng, through `rquickjs` 0.14.0 pinned to that version
(`=0.14.0`), with `default-features = false, features = ["std"]`, so
that no allocator feature replaces QuickJS's own, under which the heap
limit would do nothing (`rquickjs` documents that). `rquickjs-sys`
carries QuickJS-ng's C source, 0.16.2, and builds it with `cc`; that is
the vendoring M24.2 asks for, the source pinned by the crate's version
and its checksum in `Cargo.lock` rather than copied into the
repository. All three crates are MIT, and they add no other crate to
the tree: `hashbrown` 0.17 and `cc` are in it already.

The choice is the roadmap's for the reason M24.5 gives: a script that
loops or allocates without end must fail as a script, never take the
engine down. QuickJS-ng's interrupt handler and heap limit do exactly
that, measured above; Boa can do neither (no limit on its heap, and a
budget that a call from a native function escapes), and adding them is
a fork of a large engine. The cost is C: QuickJS-ng is memory-unsafe
code facing the page, as `aws-lc` is C facing the network. Risk R2
already says no untrusted script runs before the isolation of M33 and
M34, and the scripting switch below holds to that. The trait keeps the
door open: Boa, or SpiderMonkey at M29's WebAssembly decision, would
implement it without the engine changing.

**A crate of its own (M24.1).** A new leaf crate, `js`, holds the
`ScriptEngine` trait, the types that cross the boundary, the limits,
and the QuickJS-ng backend. It depends on no crate of the engine's, so
nothing in it can reach the document but through the host it is handed
(M24.4's rule, made by the dependency graph rather than by review).
The layering becomes `engine -> js`, a leaf beside `images`, and
`docs/architecture.md` says so. `js` itself writes no `unsafe`: it uses
`rquickjs`'s safe interface alone and says so with
`#![forbid(unsafe_code)]`; the unsafe code is `rquickjs`'s and
QuickJS-ng's, third-party, pinned and fuzzed through the crate's entry
points. The architecture's rule on `unsafe` (M33.5's "unsafe exists
only in FFI crates") is written now: no crate of the workspace writes
`unsafe` but `shell`'s platform calls, each with a safety argument;
FFI is reached through a third-party crate's safe interface.

**The trait (M24.1).** The engine sees the script engine as two traits:

```rust
pub trait ScriptEngine {
    /// A new realm: a global object, its heap and its limits.
    fn realm<H: ScriptingHost>(&self, limits: Limits)
        -> Result<Box<dyn Realm<H>>, ScriptError>;
}

pub trait Realm<H: ScriptingHost> {
    /// Parses and runs a classic script; its completion value.
    fn run(&mut self, script: &Script<'_>, host: &mut H)
        -> Result<Completion, ScriptError>;
    /// Runs the queued promise jobs (HTML 8.1.7.3); their errors.
    fn checkpoint(&mut self, host: &mut H) -> Vec<ScriptError>;
    /// Bytes of heap in use, and a collection asked for.
    fn heap(&self) -> usize;
    fn collect(&mut self);
    /// Why the realm can run nothing more, if it cannot.
    fn dead(&self) -> Option<Death>;
}
```

`Script` is the source text, the URL errors name, the line its text
starts on in its document, and whether its errors are muted. A
`Completion` is the value a script ends with, kept only as far as a
`javascript:` URL needs it: `undefined`, a string, or something else.
"Compile, eval, call" (M24.1) are `run` for now: a script is compiled
and run in one call, as HTML's "create a classic script" and "run a
classic script" do back to back; calling a function the page gave the
engine (a listener, a timer's callback) comes with what holds those,
M26, as a third method. The garbage collector's integration point is
`heap` and `collect` now, and the release of the handles a realm holds
to nodes when M25 gives scripts nodes. Error sinks are the errors these
methods return and the console below.

**The boundary (M24.4).** Everything a script can do to the engine is a
method of one trait, `ScriptingHost`, whose arguments and results are
plain Rust data: strings, numbers, and from M25 node handles, which are
numbers the host resolves and checks, never references. No object of
the engine's is reachable from the realm, and no JavaScript value
leaves it: each crossing converts, in `js`, at the call. In M24 the
host has three methods: `console(level, message)`, `title() -> String`
and `set_title(String)`. A run takes the host by `&mut`; the realm
moves it into the slot its native functions read, for the run alone,
and moves it back before returning, so a native function can reach the
host only while a script the engine started is running, and only
through that trait. The engine's host owns the document for the run
(moved out of the engine and back), so a script's changes go through
the same `dom::Document` methods and journal as any other change.

**What scripts can reach in M24.** The global object is `window`, also
`self` and `globalThis`, with ECMAScript's built-ins as QuickJS-ng has
them, which leave out `Intl` and `Temporal`. *(corrected)* QuickJS-ng
adds `queueMicrotask`, `atob`, `btoa` and `DOMException` of its own;
its `performance`, an unrounded clock, and `SharedArrayBuffer` are taken
off the global object. Of the platform:
`console.log`, `info`, `warn`, `error` and `debug`, with the Console
Standard's formatter (`%s`, `%d`, `%i`, `%f`, `%o`, `%O`, `%c`, the last
dropping its styles); and `document`, an object with `title` alone,
which reads and writes as HTML section 3.1.7 says (the first `title`
element's text with white space collapsed; a set making a `title` in
the head if there is none, and doing nothing without a head). Nothing
else of the DOM, no events and no timers: those are M25 and M26.

**Limits (M24.5).** Named constants in `js`, each also a field of
`Limits` so tests can make them small:

- `MAX_SCRIPT_HEAP`, 256 MiB: a realm's heap. An allocation past it
  throws a catchable `InternalError`. One that escapes a run leaves the
  realm *dead* (`Death::Heap`): what the script held is still held, so
  the next allocations would fail too (measured, with the array a
  global), and the engine drops the realm and runs no further script in
  that document, which stays as it was drawn. *(corrected)* Dead only
  when, after the failed run and a collection, the heap still holds
  more than three quarters of the limit (`DEAD_HEAP`): a script that
  caught the error and let go of what it held goes on. A realm that
  cannot be made at all, its heap too small for one, stops the
  document's scripts as a dead one does.
- `MAX_SCRIPT_STEPS`, 10,000: interrupt calls a run may take, each
  about 10,000 of QuickJS-ng's operations; 2 to 5 seconds of running
  code on this machine. Past it the script ends with an uncatchable
  error, as HTML section 8.1.4.5 allows ("abort the script without an
  exception"), reported as the page's error; the realm lives, and the
  next script has a budget of its own. A run, a microtask checkpoint
  and a `javascript:` URL each have one. A budget of operations rather
  than of time, so that a test of it gives the same answer on any
  machine.
- `MAX_SCRIPT_STACK`, 256 KiB: QuickJS-ng's stack limit, always set,
  since its default is the whole of a Windows main thread's stack. Past
  it a catchable `RangeError` is thrown. The engine's thread has 1 MiB
  (the main thread on Windows) or more, and runs scripts near its base,
  so 768 KiB stay for the host's calls and the frames below.
- `MAX_CONSOLE_ENTRIES`, 1,000, and `MAX_CONSOLE_MESSAGE`, 16 KiB, in
  the engine: what a document's console keeps, the oldest dropped, a
  message cut at a character boundary.
- `MAX_SCRIPT_BYTES`, 32 MiB, in the engine: an external script's body;
  past it the fetch fails and the script is an error.

A string crossing the boundary is bounded by the heap it came from.

**Errors and the console (M24.1's error sinks).** A `ScriptError` says
what failed: a syntax error, an uncaught exception, the step budget, the
heap, with the message, URL, line and column. The engine reports each
as HTML section 8.1.4.6 says, without the `error` event, which waits
for events (M26): to the document's console, as an error entry
beginning `Uncaught` *(corrected: but the step budget's, "Script ran too
long and was stopped" and where)*. A script that could not be loaded is
a console error too, "Failed to load script <url>: <why>", and so is a
script a policy refused, "Content-Security-Policy refused <what>:
<directive>"; neither counts among the page's script errors. The errors of a script fetched from another origin
are *muted* (section 8.1.4.6): "Script error.", no URL, no position,
since without CORS (M27) no response is CORS-same-origin. A promise
rejected with no handler by the end of a checkpoint is reported as
`Uncaught (in promise)` (section 8.1.4.7). The console is a list of
entries, level and text, that the engine keeps per document and the
shell takes (`Engine::take_console`) and prints to its standard error,
`console: <level> <text>`, as devtools would show it (M36).

**The script-error page state (M24.7).** `Engine::scripts()` reports
the shown document's scripting: off, running, or stopped (its realm
dead), with the count of errors reported. A realm that dies posts the
status bubble's notice, "Scripts stopped on this page: out of memory",
and the document stays as drawn. *(corrected)* The notice is posted once
the document whose realm died is shown, not over the page shown while
it loads. A script error does not change
`LoadState`: the page loaded; its script failed.

**Scripting is off by default (risk R2).** The roadmap orders phases D
and E: no untrusted script runs before E-grade isolation, M33 and M34.
So the engine runs no script unless asked:
`Engine::set_scripting(true)`, which the shell sets from a new flag,
`--enable-scripts`, and tests set. HTML's *scripting is disabled*
holds otherwise, and everything that depends on it follows the switch:
the parser's scripting flag
(with it, `noscript` is a raw-text element whose content is text; the
UA sheet then gives `noscript` `display: none`, which leaves its
computed value different from the reference browser's `inline` until
something can read it, M25), the `scripting` media feature (`enabled`
or `none`), and `javascript:` URLs. With scripts on, the rest of R2's
posture holds by what the engine lacks: no cookies (M27), no storage
and no profile on disk (M28), and a page's own CSP enforced, failing
closed where it is not understood (below).

**Parsing as bytes arrive (M24.8).** Three pieces, each its own step.

- *The tokenizer* becomes HTML section 13.2.5's state machine: every
  state, the script data states with their escapes among them, RCDATA,
  RAWTEXT, PLAINTEXT and CDATA sections, and the full named character
  reference table (2,231 names; today's has HTML 4's) *(corrected: and
  the processing-instruction states of the Standard of 7 October 2026,
  whose tokens the tree builder drops)*. It is fed text in
  pieces (`feed`, then `end`), keeps its state between them, and gives
  the same tokens whatever the pieces; carriage returns are normalized
  across a piece's edge. Which state an element's content starts in is
  the tree builder's to set, as section 13.2.6 does (a `script` in HTML
  content, not in SVG; `noscript` only with the scripting flag). It
  counts lines, so that an inline script's errors name its line in the
  document. Parse errors are not reported: the tree is what matters.
- *The tree builder* writes into a `dom::Document` as it goes: an
  element is inserted into its parent when its start tag is processed,
  and the stack of open elements holds node ids *(corrected: `dom`
  gained `create_element_with`, `append_data` and `append_new`, the last
  appending a fresh node in constant time, its depth the parser's to
  bound, since `append_child`'s walk of the ancestors made 200,000 nested
  elements take 9 s)*. Its insertion modes
  stay the subset they are today (section 2 says what is missing); what
  changes is where the tree lives and that the builder can stop. A
  `Parser` holds the tokenizer and the builder: `feed(text)`, `end()`,
  and `parse(&mut Document, slice)`, which processes at most `slice`
  tokens and returns why it stopped: input wanted, the slice spent,
  done, or a `script` end tag processed, with the element's id and the
  line its text began on. The engine prepares the script there before it
  calls `parse` again; while a parser-blocking script is pending it does
  not. The insertion point that `document.write` needs is the input
  stream's to keep, and is built when `document.write` is (M25).
  `html::parse_document` stays, a `Parser` run to its end, for the
  pages the engine makes, the tests and the fuzz targets.
- *Bodies arrive in pieces.* `net` gains a fetch that hands over the
  response's head and then its body in chunks as they are read, content
  codings decoded as a stream and the body limit counted as it grows;
  the blocking `fetch` is that one gathered. The loader passes the
  chunks to the engine over the channel the results take now, waking
  it. The engine decodes them with the `encoding` crate's streaming
  decoder (which gives the same text in pieces as at once) after
  sniffing on the first 1,024 bytes or the whole body if shorter, and
  feeds the parser. A `<meta>` that changes a tentative encoding late
  starts the parse again from the first byte with the new one, the
  document built so far, its realm and its scripts' effects dropped,
  which is HTML's reload in place (section 13.2.3.4); a script may
  then run twice, as it does in the reference browser. *(corrected)*
  The console is kept: what ran, ran.

The engine parses a slice (`PARSE_SLICE` tokens, its value measured at
the step that builds it) each time it is polled with input waiting, and
wakes itself again when there is more, so the window draws between
slices. The document being loaded is shown when it stops being
*render-blocked* (section 3.1.6): it has a body, and no style sheet or
script in its render-blocking set is pending; or its parse is done. That
is today's rule, the head's style sheets, with the head's
parser-blocking scripts added. From then on frames are drawn as it
grows: the parser's insertions are journal entries like any other
change, so style sheets and images are found and the boxes restyled and
laid out by the incremental engine as for a script's change. A document
still parsing when it is left is not kept in the back-forward cache.

**The script element (M24.3).** HTML section 4.12.1.1's "prepare the
script element", for the elements the parser inserts (scripts make none
until M25):

- *Types.* No `type` and no `language`, an empty one, or a JavaScript
  MIME type essence (net's `is_javascript`) is a classic script.
  `module`, `importmap` and `speculationrules` are recognized and not
  run (section 2); anything else is not a script. A classic script with
  `nomodule` is skipped, as browsers that run modules skip it.
- *Inline scripts* run when their end tag is processed, unless a style
  sheet is blocking scripts (a parser-made `style` or `link` sheet whose
  media matches and that is still being fetched): then the script is the
  pending parsing-blocking script and runs once the sheets are in.
- *External scripts* (`src`) are fetched at once. Without `async` or
  `defer` the script is the pending parsing-blocking script: the parser
  stops until it has arrived and no style sheet blocks scripts, then it
  runs and the parser goes on. With `defer` it joins the list run in
  order when parsing ends; with `async` the set run as each arrives.
  The list of scripts run in order as soon as possible is for scripts
  that scripts insert, M25.
- *Running a script* is HTML's "run a classic script": the host given
  the document, the step budget renewed, the errors reported, and a
  microtask checkpoint after it, which is when the reference browser
  ran the probe's promise reaction. `document.currentScript` waits for
  the property, M25; the `load` and `error` events for M26.
- *Render blocking.* A parser-blocking script in the head blocks
  rendering until it has run, as the head's style sheets do now.
  *(corrected)* That needs no code of its own: the parser waits at the
  script, before any body, and a document is shown once it has one.
- *Loaded.* `LoadState::Loaded` waits for the parse to end and the
  deferred and async scripts to have run, as the load event does.
  *(corrected)* Stopping the load drops the scripts on their way, and
  the parser runs none it meets after, as HTML's abort does.

**Fetching scripts.** A new `Destination::Script` in `net`, with
`Accept: */*`, `Request` built as the loader builds a style sheet's
(the document's initiator, its address space, `MAX_SCRIPT_BYTES`).
Mixed content blocks an `http:` script on an `https:` page, as for
style sheets since M14. The response is the script if it is `ok` and
Fetch section 2.10's "should response be blocked due to its MIME type"
lets it through (no `audio/`, `image/`, `video/` or `text/csv` type),
and under `X-Content-Type-Options: nosniff` only if its type is a
JavaScript MIME type. It is decoded by BOM, else the `charset` of its
`Content-Type`, else the element's `charset` attribute, else the
document's encoding. A script whose final URL's origin is not the
document's has muted errors. *(corrected)* So has one any of whose URLs
was another origin's, a redirect away and back included, as Fetch's
response tainting keeps it; a `data:` script has not. A failed fetch is a script whose result is
an error: nothing runs, and nothing is reported but the console's
"Failed to load script" line, since the `error` event is M26's.

**Content Security Policy (M24.6).** CSP's `script-src` and what it
falls back to, so that a page that says "no inline scripts" or "scripts
from here only" is obeyed from the first script. Parsing and matching
live in `net` (`net::csp`), beside the URLs and origins they compare;
the loader applies them.

- *Delivery.* Every `Content-Security-Policy` header of the document's
  response, each value a list of policies split on commas; and a
  `<meta http-equiv="Content-Security-Policy">` the parser inserts into
  the head, which applies to what comes after it. The `-Report-Only`
  header is ignored: reports are M33.1's, and a policy that only
  reports blocks nothing. *(corrected)* A meta's policy is taken in the
  first time a check looks after the parser inserted it: before the
  next script, script request or `javascript:` URL.
- *Directives.* For a script element, `script-src-elem`, else
  `script-src`, else `default-src`; for a `javascript:` URL the same,
  as an inline check. Every policy must allow.
- *Source expressions.* `'none'`, `'self'`, `'unsafe-inline'`,
  scheme sources (`https:`) and host sources (`*.example.com:*/path`,
  ports and paths as section 6.7.2 matches them), with `*`. The nonce
  and hash sources and `'strict-dynamic'` are M33.1's; until then they
  match nothing, and their presence disables `'unsafe-inline'` as
  section 6.7.3.2 says. The policy is never looser than the page asked:
  what is not understood blocks. `'unsafe-eval'` is not enforced:
  QuickJS-ng has no hook for code made from strings, so `eval` and
  `Function` run whatever the policy (M33.1, which needs the hook in the
  binding). *(corrected)* A document keeps at most 64 policies
  (`MAX_POLICIES`); one more blocks every script. An IP address matches
  no host source, as section 6.7.2.10 says.
- *Where it is checked.* An inline script before it runs; an external
  script's URL before it is fetched and at every redirect (Fetch's
  "should request be blocked by Content Security Policy", in `net`'s
  per-hop checks, from the policies the request carries); a
  `javascript:` URL before it runs. A blocked script does not run, and
  the console says which directive blocked what. *(corrected)* The
  post-request check is not made: with no service worker, the
  response's URL is the last hop's, which the pre-request check saw.

**`javascript:` URLs.** With scripting on, following a `javascript:`
link (or typing one, which the address field already refuses) runs its
percent-decoded text as a classic script in the shown document, after
the CSP check, with its own step budget. If its completion is a string,
HTML replaces the document with that string as HTML; that is scripts'
part in navigation and waits for M26.6. With scripting off, the link
does nothing, as now.

**One realm per document.** A document gets its realm when its first
script runs, with the limits above, and keeps it while it is shown or
kept in the back-forward cache, where its garbage is collected before it
is stored; going back to it runs no script again (its `pageshow` is
M26's). A document left while parsing, or whose realm died, is not
kept.

**Tests.** Unit tests in `js` for every limit, error kind, the
formatter, the boundary and a host that records its calls; in `html`
for the tokenizer and the parser in pieces; engine tests for the order
of scripts, the parser pause, stylesheets blocking scripts, CSP, the
switch, the console and the error state. html5lib's tokenizer tests,
copied at a pinned commit with their licence into
`crates/html/tests/html5lib/` (MIT; *(corrected)* 1.7 MB, most of it
`namedEntities.test`), run as
`html`'s tests: the tokenizer's conformance suite. Fuzz targets:
`html_parse` feeds its input in pieces and checks the tree against the
whole; a new `script` runs any input as a script under small limits,
against a host that records, and checks that the limits hold and
nothing panics; a new `csp` parses and matches any policy.

## 2. Not in this milestone, said rather than hidden

Each item that no milestone holds is registered on the G list at the
documents step; the number follows each.

- *The DOM's interfaces*: nodes, elements, `document` beyond `title`,
  `document.currentScript`, `document.readyState`, `innerHTML`; and
  `document.write` with the insertion point. M25 (`document.write` is
  added to M25.1 at the documents step).
- *Events*: `load`, `error`, `DOMContentLoaded`, `pageshow`; timers and
  the event loop's tasks. M26.
- *A string result of a `javascript:` URL replacing the document*.
  M26.6.
- *Module scripts, import maps and speculation rules.* G41.
- *`Intl` and `Temporal`*, which QuickJS-ng does not implement. G42.
- *Subresource Integrity* (`integrity`). G43.
- *The `crossorigin` attribute, CORS and unmuted cross-origin errors*.
  M27.
- *CSP beyond `script-src`*: nonces, hashes, `'strict-dynamic'`,
  `'unsafe-eval'`, the other directives, reporting. M33.1.
- *The tree builder's missing insertion modes*: the table modes, the
  active formatting elements and the adoption agency, foster parenting,
  "in head noscript", templates and foreign content beyond self-closing
  elements; comments and doctypes in the tree. G44 ("in head noscript"
  is already G12's).
- *The speculative parser and the preload scanner.* M38.3.
- *Freeing detached nodes*, which waits for scripts to hold nodes. M25.
- *Process isolation of scripts.* M34.
- *A debugger, source maps and the console's other methods.* M36.
- *Structured fuzzing of API call sequences* (T2.4): there is one call
  in M24; M25 has the API to fuzz.

## 3. Order of work

Each step is a vertical slice: build, tests, clippy green and a smoke
run before the next, with its sabotage pass.

1. *The `js` crate.* The traits, the boundary types, `Limits` and its
   constants, the QuickJS-ng backend: realms, `run`, the checkpoint,
   errors and their kinds, the interrupt and heap limits and a dead
   realm, the console's formatter, `document.title` through the host;
   the dependency and `deny.toml` changes; the layering; the `script`
   fuzz target. M24.1, M24.2, M24.4, M24.5.
2. *The tokenizer.* Section 13.2.5's state machine, fed in pieces, the
   full named reference table, line counting, the tree builder's state
   switches; html5lib's tokenizer tests. M24.8.
3. *The tree builder into the document.* The `Parser`, writing into
   the arena, the open elements in the tree, the scripting flag, the
   pause at script end tags, `parse_document` on top; `html_parse` in
   pieces. M24.8.
4. *Bodies in pieces.* `net`'s streaming fetch, content codings as a
   stream, the limits as the body grows; the loader's chunks. M24.8.
5. *Parsing as bytes arrive.* Streaming decoding and sniffing, the
   restart on a late encoding, slices and self-waking, showing a
   document once it is not render-blocked, frames as it grows, sheets
   and images found through the journal. M24.8.
6. *Scripts run.* The switch and `--enable-scripts`, `noscript` and the
   `scripting` media feature, a realm per document, inline classic
   scripts at their end tags with checkpoints, the host, the console,
   muted errors, a dead realm and the page's script state. M24.3,
   M24.7.
7. *External scripts.* `Destination::Script`, the response checks and
   decoding, parser-blocking, `defer` and `async`, style sheets
   blocking scripts, render blocking, `Loaded`, `javascript:` URLs.
   M24.3.
8. *Content Security Policy.* `net::csp`, the header and the `meta`
   element, the checks before running, before fetching and at each
   redirect; the `csp` fuzz target. M24.6.
9. *The web-platform tests.* The subset's rule taking tests whose
   scripts use only what M24 gives, and every failure in
   `expectations.txt` with its reason. *(corrected)* No test in the
   subset's directories uses `document.title`: the rule takes inline
   scripts that use nothing of the platform but `console` and
   `document.title`, and event handlers whose events never fire while a
   test is drawn, which brought in 14 CSS2 generated-content tests; the
   runner runs scripts, and the two `scripting` tests pass.
10. *Golden cases.* Pages from 116 with scripts on (`noscript`, the
    `scripting` feature, a script's effect on the title), captured from
    the reference browser. *(corrected)* Pages 116 to 118, the third the
    order scripts run in; their titles are drawn as boxes of en dashes,
    half an em each, so that the widths are whole pixels in both
    browsers.
11. *Acceptance.* `console.log("hello")` and `document.title = "x"` from
    an inline script, a script error, a realm out of memory, in the
    shell; a benchmark, `benches/scripts.rs` (a realm, running scripts,
    a streamed load); T4.3, the parser's rewrite among what it watches;
    fuzz runs of `script`, `csp`, `html_parse` and the targets that lay
    pages out. M24.7.
12. *The documents.* This note corrected to the code; the
    architecture's entry, layering and `unsafe` rule; the roadmap's
    items ticked, D1 recorded, M25.1 gaining `document.write`, and
    G41-G44 registered; `docs/perf-budgets.md`'s numbers.
