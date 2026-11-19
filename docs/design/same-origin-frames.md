# Design note: frames of one origin share an agent (G90)

Status: approved 2026-11-18; phases 1 and 2 built (2026-11-19), 3 to
6 to come. Phase 2 as built, beyond section 1: a window handed out for
a frame before its realm existed forwards to the frame's window once
it does; a window's `postMessage` called from another realm of the
agent is that realm's post (its `source`, its serializing); a
document's realm is made when it runs a script (made at every frame's
commit, a page of many scriptless frames paid some 25 ms and 3 MB a
frame, and the frames fuzz target ran 30 times slower: until phase 3
makes the realm as it is first reached, a scriptless document is not
reached); and a node of another document given to a node argument,
`getComputedStyle` or an animation's target throws NotSupportedError,
said and counted (`nodesAcrossDocuments`), until phase 5.
G90 is a beta requirement (the lead's ruling, 2026-11-18): Gmail, Docs
and Drive are on the user's daily list, and sign-ins behind anti-bot
checks (Kasada on Twitch) read their own frames.

## 0. Where it stands, and what breaks

M33 made each frame an engine of its own under its parent's (D24,
`docs/design/hardening.md`, "Frames"), and every engine's realm a
QuickJS runtime of its own. M34 placed frames by site
(`docs/design/processes.md`): a frame of its parent's site stays in
its parent's renderer, a child engine, and one of another site goes to
that site's renderer. So two frames of one origin are in one process,
on one thread, but in two runtimes, and a script reaches the other
only through the proxy HTML gives across origins (`frameWindow` in the
bootstrap): its `contentDocument` is null, and anything but `window`,
`location`, `postMessage` and the like throws `SecurityError`. That is
the closed side of HTML's rule; it was G90 from the start.

What it breaks, measured (the 2026-11-18 baseline's counters, a
script's reach into its own origin's frame refused, top renderer
only): bbc 174, weather 79, Gmail, Docs and Drive 5 each, Google
Images, Grok and a YouTube watch page 2, ai-chat and Reddit 1. And
what no counter of the top saw: Twitch's Kasada frame makes an
`about:blank` child and reads `TextEncoder` from it (the anti-bot
"fresh frame" trick, for pristine built-ins), and gets a
`SecurityError` where Chrome gives it the child's `TextEncoder`.

## 1. Decisions

### The unit of sharing: an origin-keyed agent cluster

HTML groups the documents that may reach each other synchronously into
*agent clusters*: within a browsing context group, one cluster per
*site* by default, or per *origin* when the document asks for an
origin-keyed cluster (`Origin-Agent-Cluster: ?1`) or the browser makes
them so by default. Each cluster has one *agent*: one event loop, one
JavaScript heap, and the realms of its documents in it.

**Keyed by origin, always (recommended).** The reference browser keys
clusters by origin by default, as `docs/design/hardening.md` recorded
at M33 (to be checked against Chrome 157 in phase 2's tests), which
makes `document.domain` inert unless a page sends
`Origin-Agent-Cluster: ?0`; this browser already treats
`document.domain`'s setter as inert. Keyed by origin:

- the rule a script meets is the one HTML states for most pages now:
  same origin, same agent; no page can widen it by `document.domain`;
- the security argument is the simplest (below): nothing that is not
  same-origin ever shares a heap;
- the cost is the rare page that still sends `?0` and sets
  `document.domain` to reach a sibling subdomain: it gets today's
  `SecurityError`. A known-gaps line, and a G item if a site on the
  list needs it.

Keyed by site would follow HTML's default letter, but would put
`a.example.com` and `b.example.com` in one heap with the access check
alone between them, for the sake of `document.domain`, which the
reference browser has turned off. Not chosen.

An opaque origin (a sandboxed frame without `allow-same-origin`, a
`data:` document) is an origin of its own: an agent of its own.

### The runtime and its realms

**One QuickJS runtime for each agent, a realm (context) for each of
its documents.** QuickJS-ng holds several contexts in a runtime and
lets an object of one be used from another: the heap, the garbage
collector, the job queue and the interrupt handler are the runtime's.
So:

- an *agent* (new, `scripts::Agent`) owns the runtime, its budget and
  its heap limit, and the documents' realms are contexts in it. The
  top-level agent's heap stays 256 MiB; a frame's own agent keeps 64
  MiB; frames that join their parent's agent share its limit;
- each realm keeps its own global, its own bootstrap (interfaces are
  per realm: `frames[0].Array !== Array`, as in Chrome) and its own
  natives, bound to its own document's engine;
- the job queue is the agent's: one microtask checkpoint for all its
  realms, as HTML's agent has one;
- the realm's object wrappers stay per realm (a node has one wrapper,
  in its document's realm), and an object passed to another realm of
  the agent is that same object.

The bootstrap's cost per realm (a few MiB of heap) is paid per
document still; what sharing saves is the runtime's own (its atoms,
its shapes, its job queue) and, mostly, nothing: the gain is
correctness, not memory (see section 4).

### References between realms

Within one agent, a script gets the other document's real objects:

- `iframe.contentWindow` and `frames[n]`, `parent`, `top`, `opener`
  (none until M37's popups) return a **WindowProxy**: one per frame
  for the agent, a `Proxy` whose target is the frame's current
  `Window` when the frame's document is of the agent, and whose
  behaviour is today's `frameWindow` when it is not (another origin,
  or the frame gone). The check is made at each access, as HTML's
  WindowProxy makes it, so a frame that navigates to another origin
  turns its proxy cross-origin at once, and back;
- `contentDocument` returns the frame's `Document` when the frame is
  of the agent, null otherwise (as now);
- `window.frameElement` in a frame of the agent returns the parent's
  element, null across agents (as now).

**The hosts of an agent's documents are lent together.** Today a run
lends the realm one document's engine (`Slot<H>`). A script of the
parent calling `frameDoc.querySelector()` calls the frame realm's
function, whose natives need the frame's engine. So a run in an agent
lends every engine of the agent: the run is made from the engine at
the agent's top (the highest engine whose document is of the agent:
the tab's top for most pages, a frame root for a frame of another
site's subtree), which owns the others as child engines and can lend
them all for the run's length. A child engine never starts a run of
its own while it is in an agent above it; its tasks are its agent's,
run from the agent's top (below).

**A node of one document given to another realm's method** (the
parent's `body.appendChild(frameDoc.createElement('p'))`) is HTML's
adoption across documents. The bootstrap's `handleOf` knows a node's
realm; a node of another document of the agent is moved by the
engines' adoption across documents (the D67 machinery for documents
beside the page, extended across engines: the subtree serialized out
of one engine's document and made in the other's, its wrappers kept).
Until that phase lands, such a call throws a `NotSupportedError` with
a console line, and the sweep counts it: no silent wrong answer.

### Which agent each document lands in, and when

- **The initial `about:blank`** of a new `iframe` is made synchronously
  when the element is connected, in its *creator's* agent (the
  document whose script connected it, else its parent's), with the
  creator's origin: `blank.contentWindow.TextEncoder` works right after
  `appendChild`, as in Chrome. It is a child engine in the creator's
  renderer, which M34 already gives it ("frames of an opaque origin
  and the initial about:blank inherit their creator's place").
- **`about:srcdoc`**: the parent's origin, so the parent's agent.
- **`javascript:` URLs**: run in the frame's current document's realm,
  as now; the document they may make keeps that origin.
- **`blob:`**: the blob's origin (its creator's); its creator's agent
  when that is the frame's parent's, else the agent of that origin in
  the tab, else a new one.
- **`data:`**: an opaque origin, a new agent, in the creator's
  renderer (M34).
- **A navigation to a URL**: the agent of the response's origin in the
  tab if there is one in the renderer the site goes to, else a new
  agent there.

### Process placement, and why nothing gets weaker

Placement stays M34's: a renderer per site, a frame of another site in
that site's renderer. An agent never spans processes: an agent is of
one origin, and an origin is of one site. Two agents of one site
(`a.example.com` and `b.example.com`) share a renderer, as now, but not
a runtime.

The security argument: a runtime is shared only by documents of one
origin, which HTML already lets reach each other synchronously; the
access check in the WindowProxy is the same check the bootstrap makes
now, made per access instead of once. Nothing of another origin is in
the heap of a shared runtime, and nothing of another site is in the
process. A compromised renderer of site S could read every origin of
S, as it can today (its child engines share its address space). So the
change opens exactly what HTML's same-origin rule opens, and the
cross-site boundary (the process) and the cross-origin boundary (the
runtime, the proxy) are untouched. The threat model in
`docs/security.md` gains a row: "same-origin frames share a heap
(G90): the boundary is the origin, as HTML's".

### Navigation between agents

A frame that navigates keeps its number and its WindowProxy (what
scripts hold). Its old document's realm is detached: its global is no
longer the proxy's target, and scripts that held its objects keep
them, which the runtime keeps alive (Chrome keeps a detached realm the
same way). The new document's realm is made in the agent its origin
lands in (above): the same agent (a same-origin navigation), another
agent in the same renderer (same site, another origin), or another
renderer (another site, where the parent keeps a remote frame, as
M34). Coming back to the agent's origin makes a new realm in it, and
the proxy's target is that realm's global again.

### The event loop

An agent has one event loop. Its documents' tasks (timers, fetches'
results, events, messages) are taken in the order they were queued,
across its documents, from the agent's top, which runs them with every
engine of the agent lent, each followed by the agent's microtask
checkpoint. Within a renderer, agents are run in turn as the engines
are now (each engine's `run_tasks` from its parent's), and a task
slice is the agent's.

The rendering update stays per document, in the order HTML gives (the
frames' after their parent's, depth first): animation frame callbacks,
the observers and the paint, as `frame_at` does for each engine, with
the agent's engines lent while any of its documents' callbacks run.

### What changes in the code

- `js`: `ScriptEngine::realm` gains `realm_in(agent, ...)`; the runtime
  moves from `QuickJsRealm` into `Agent`; the interrupt handler, the
  budget, the heap limit, the job queue and the promise hooks become
  the agent's; the debug guard of natives and `clear_stale` stay per
  realm. `Realm::run` takes the agent's hosts (`AgentHosts<H>`, a slot
  per realm) instead of one host.
- The bootstrap: `windowFor` returns the agent's WindowProxy for a
  frame; `contentDocument`, `frameElement`, `parent`, `top` as above;
  `handleOf` refuses (until adoption) a node of another realm.
- `engine`: an `Engine` knows its agent (its document's origin's, in
  its renderer); the agent's top lends its engines for a run; a child
  engine of the agent queues its tasks to the agent; `frames.rs`'s
  frame tree records each frame's agent.
- The inspector: `console.evaluate` runs in the top realm of the agent
  as now; a frame's realm is named by its frame number.
- The sweep: `frameRefusedSameOrigin` and `frameDocumentSameOrigin`
  should go to zero; a new counter, `frameAdoptRefused`, until
  adoption lands.
- The tests: `frame_script_tests.rs` asserts today that same-origin
  frames are cross-origin (the "closed side"); those assertions turn
  into the open side, against Chrome's answers, and the cross-origin
  ones stay. The WPT runner's expectations for `html/browsers` and
  `html/document-isolation-policy`-like directories are re-run.

## 2. Measures

**Memory.** Small, either way: a same-origin frame keeps its own
realm and bootstrap (a few MiB of heap each), and loses only its own
runtime's overhead. The pages that grow (weather's 13 processes after
the load) grow by frames of *other sites*, which G90 does not touch;
fewer processes for them would be a different change (a renderer for
ad-like frames of many sites, which weakens site isolation and is not
proposed). The note claims no memory win.

**CPU.** One event loop per agent instead of one per frame: no change
in work. Synchronous reach replaces what pages did by `postMessage`
fallbacks, where they had them: a small win, unmeasured.

## 3. Phases, estimates and tests

Each phase is tested against Chrome: the expected values are Chrome's
answers on the same page, recorded by the reference capture.

1. **The agent and its runtime** (2 to 3 days). `Agent` owns the
   runtime; realms are contexts in it; a top-level document's realm is
   an agent of one realm, so nothing visible changes. Tests: every
   current test passes; two realms of one agent share a heap limit and
   one microtask checkpoint (a promise of one settles at the other's
   checkpoint); the heap limit and the budget stop both.
2. **Same-origin frames join their parent's agent, and reach each
   other** (4 to 5 days). WindowProxy, `contentDocument`,
   `frameElement`, `parent`, `top`, `frames[n]`; the agent's hosts
   lent together. First test, the Kasada case: a frame of `b` makes an
   `about:blank` child and reads `contentWindow.TextEncoder`, a
   function, with `Function.prototype.toString` of it native; then a
   parent reading its same-origin frame's `document.title`, calling a
   function the frame defined, and the frame calling `parent.f()`;
   cross-origin cases unchanged. Gmail's, Docs' and bbc's refused
   counts go to zero in the sweep.
3. **Which agent each document lands in** (2 days). The synchronous
   initial `about:blank`, `srcdoc`, `javascript:`, `blob:`, `data:`,
   and navigation between agents with the proxy turning
   cross-origin and back. Tests against Chrome for each.
4. **The agent's event loop** (2 days). Tasks across an agent's
   documents in queue order; the rendering update with the agent lent.
   Tests: interleaved timers of parent and frame log in Chrome's order;
   a frame's `requestAnimationFrame` reading its parent's layout.
5. **Adoption across documents** (2 to 3 days). A node of one
   document of the agent appended to another's. Tests against Chrome:
   `appendChild`, `importNode`, `adoptNode` across frames, the node's
   `ownerDocument` and listeners after.
6. **Counters, inspector, WPT and the sweep** (1 day). The sweep's
   refused counters at zero for same-origin; WPT re-run and
   expectations.

About three weeks in all. Phases 1 and 2 are the beta's need (the
Kasada case, Gmail, Docs); 3 and 4 make it right; 5 can follow if no
site on the list needs it first.

### Phase 3 (b): a realm made as it is first reached

Approved by the lead 2026-11-19, with the amendments folded in below
(policy container, sandboxed and cross-origin inserts, identical
globals, reference counts, the sweep's medians first).

What it fixes: a document of the agent that ran no script has no realm
(fix (a), phase 2), so a script of the agent that reaches it (a
frame's `contentWindow`, `frames[n]`, `parent`, `top`, a name) gets
the cross-origin proxy and `SecurityError`; and an `iframe` a script
inserts has no document at all until the frames pass after the run,
where Chrome gives the initial `about:blank` at once (the Kasada case:
`document.body.appendChild(f); f.contentWindow.TextEncoder`). The
2026-11-19 sweep found phase 2 left the daily sites' refused counts
where they were (bbc 147 to 178, weather 104 to 122, Gmail, Docs and
Drive 5): each frame reached had no realm then. This is what moves
them.

**When.** In the native that answers the reach (`frameWindow`,
`parentWindow`, the frame by name or index), when the agent has no
realm of that frame and the frame's document is of the agent. Never
eagerly: fix (a)'s numbers (25 ms and 3 MB a scriptless frame, the
fuzz target 30 times slower) stand.

**Whose document.** Two cases, both answered from the agent, without
reaching into the engine during the run:

- a document that exists (a scriptless page, a scriptless parent, a
  frame's initial `about:blank` its engine showed): when its engine
  lends around (`lend_around`, `with_frame`), a document of the agent
  with no realm is lent too, into the agent's table of unborn
  documents by frame (`AgentCore::unborn`, a host by frame number),
  and taken back with the rest. A host lent so is the same moves as
  any lending (`DocumentHost::lend`). Its cost is measured on bbc and
  weather, the sites with the most such frames (signed-in Gmail cannot
  be swept): over a few percent of load, the lead hears before it is
  kept;
- an `iframe` inserted during the run, whose engine is made only
  after it (`make_frame`): the reach asks the reaching realm's host,
  lent now, for a new host of an `about:blank` of its origin
  (`ScriptingHost::blank_frame(frame)`, new); `make_frame` takes that
  document and its realm from the agent (`Agent::take_made(frame)`)
  instead of making its own, so the document the script touched is
  the one the frame shows until its first navigation.

The realm so made is the frame's: its engine's `Scripts` takes it at
the take-back (`Agent::take_made(frame)` where it has none), and its
host back from it, as from any lent realm.

**A blank's policies are its creator's.** The blank `blank_frame`
makes, like the unborn initial `about:blank` (`show_blank`, which
`Page::inheriting(creator)` already gives them), carries its
creator's policy container: its CSP, its sandbox flags (the element's
and the creator's) and its referrer policy, as Chrome's does. The new
realm is guarded as its document's policies say before the reach
returns (the `Task::GuardCode` step, run in its install while its host
is lent), so that a page under a strict `script-src` cannot insert a
blank, reach it, and run `eval` or an inline script through it.
Chrome, probed 2026-11-19 (a parent under `script-src 'self'`, its
blank inserted and reached in the same task): `w.eval('1')` and
`new w.Function(...)` throw `EvalError`; a script element with text
appended to the blank's body does not run.

**What an insert reached in the same task shows.** Chrome, probed
2026-11-19:

- `src` of another origin: the initial `about:blank`, of the
  creator's origin and readable (`<html><head></head><body></body>`);
  once the navigation commits, the same `WindowProxy` answers as
  another origin's (`SecurityError`, and a property set on it before
  reads `SecurityError`), while a `Document` object kept from before
  still reads as the blank. So the blank's realm keeps no path to the
  new document: the frame's realm by frame number is the new
  document's (or none, it being in another renderer), and the proxy
  forwards by frame, not by realm;
- `srcdoc`: the initial `about:blank` in the same task, the
  `about:srcdoc` document after, through the same `WindowProxy`;
- `src` of its origin: the blank, then the page;
- `sandbox` without `allow-same-origin`: an opaque origin, so
  `blank_frame` refuses it and the reach gives the cross-origin proxy
  (`SecurityError`; `contentDocument` null), as Chrome.

**The calls, in `quickjs/context.rs` (the module that allows
`unsafe` already; its reviewed blocks go from one to three).** A native
runs inside the reaching realm's `Context::with`, which holds the
runtime's lock (`runtime.inner.lock()`, a `RefCell` borrow on this
thread) for the whole closure, so rquickjs's `Context::custom`, which
takes that lock, would panic. So, with the lock held by the caller:

1. `qjs::JS_GetRuntime(caller_ctx)`, checked to be the runtime of an
   anchor `Context` of the agent (rquickjs's `get_runtime_ptr`), whose
   `Runtime` owns the new context: the agent's runtime;
2. `qjs::JS_NewContextRaw(rt)`, null being out of memory;
3. `qjs::JS_AddIntrinsicBaseObjects`, then the intrinsics rquickjs's
   `intrinsic::All` adds, in its order (`Date`, `Eval`,
   `RegExpCompiler`, `RegExp`, `JSON`, `Proxy`, `MapSet`,
   `TypedArrays`, `Promise`, `JS_AddPerformance`, `WeakRef`,
   `DOMException`), then `JS_AddIntrinsicAToB`, as `new_context` does
   now, each nonzero return an allocation failed;
4. `Ctx::from_raw(ctx)` for the bootstrap's install, valid inside the
   held lock and not kept past the native's return;
5. `Context::from_raw(ctx, anchor.runtime().clone())` for the realm to keep:
   its `with` is used only by the realm's own runs, never nested in
   another realm's (as now: no realm enters another's `with`).

One `unsafe` block holds 1 to 3 and 5 (`new_context_locked`);
`Ctx::from_raw` is the second (`with_locked`, it is `unsafe` in
rquickjs), with the module's first, the base64 call, three in all
(built: the note first said two, counting only the new ones). Each
refuses a context of another runtime. The safety argument: the caller's
`Context::with` holds this runtime's lock on this thread; the context
is fresh and nothing has run in it; the `Ctx` does not outlive the
native call; the owning `Context` takes the one reference
`JS_NewContextRaw` gave.

The intrinsics copied by hand go stale if rquickjs changes `All`: a
test makes one realm eagerly and one by reach and compares their
global's own property names in order, those of `Object.prototype`,
`Function.prototype` and `Array.prototype`, and `typeof` of each.

**Under which lock and lending.** The runtime's lock is the
caller's; the caller's host is lent to the caller's realm (it is the
reaching run); the new realm's host is lent to it before its
bootstrap runs: the unborn host from `AgentCore::unborn`, or the
blank one from `blank_frame`. The bootstrap's install runs in the new
context, nested in the caller's call: QuickJS-ng keeps one stack of
frames per runtime, which the nested `JS_Eval` pushes on, and the
budget and interrupt check are the agent's, so the install counts
against the reaching run's budget (a few ms). The stack check
`QuickJsRealm::new` makes is made here too.

**If the bootstrap throws, or the run re-enters.**

- Out of memory or out of budget in the install: the realm is not
  made (its links leave the agent, its guards cleared, its `Context`
  dropped, which frees it at once: rquickjs frees a context whose
  runtime is locked on this thread directly); the unborn host goes
  back to the table (the blank one is dropped); the reach throws the
  error to the reaching script (`InternalError: out of memory`, or
  the budget's stop, uncatchable as now). No half-made realm stays.
- Re-entry: the install runs only the bootstrap, which reaches no
  frame; a reach of the frame being made (the agent's `making`,
  set for the install) answers as now, the cross-origin proxy, and is
  counted (a test makes it happen through a getter and sees it).
- A second reach of the frame in the same run finds the realm made
  (it is in the agent's realms by frame), so it is made once.

**Reference counts.** A test makes several realms by reach, drops some
(their frames removed), then the agent, with QuickJS's assertion
that a freed runtime holds no object on (debug builds); the frames
fuzz target runs it under ASan in CI, its input generator extended to
reach scriptless frames and inserted ones, in the insert's own task
too.

**Allowlist.** No new file: CI's grep allows
`crates/js/src/quickjs/context.rs` already; the shell's test
`unsafe_is_written_in_the_platform_module_alone_each_block_argued`
goes from `assert_eq!(context_blocks, 1, ...)` to 2, and
`docs/security.md`'s `quickjs/context.rs` paragraph from "one block"
to "two blocks", with the argument above.

**Tests.** The phase 3 gap tests flip to Chrome's lines: the Kasada
case (a frame's script inserts an `iframe` and reads
`contentWindow.TextEncoder`, a function, native by
`Function.prototype.toString`), a scriptless frame read by its
parent, a scriptless parent read by its frame, `blank write`
(`document.write` into the reached blank). New, each against Chrome's
answer: the reached blank is the document the frame shows after the
run; a parent whose CSP forbids `eval`: its reached blank's
`eval('1')` throws `EvalError`, and an inline script put in it does
not run; a sandboxed insert reached gives the cross-origin proxy; a
cross-origin insert reached in its task gives the blank, and after
its navigation a kept reference answers `SecurityError` (with its
`sub.a` twin); a `srcdoc` insert gives the blank, then the srcdoc
document; the globals of the two ways of making a realm match; the
reference-count test; a bootstrap out of heap leaves no realm and the
frame reachable later; re-entry as above; the frames fuzz target's
rate within its floor.

## 4. Not in this note

- `Origin-Agent-Cluster: ?0` and `document.domain` widening an agent
  to a site (a G item if needed).
- Popups and `opener` (M37).
- Cross-site frames in one process to save memory (not proposed).
- Shared and service workers are agents of their own, as now.
- Extensions' content scripts in frames: the extensions note, written
  against this model (isolated worlds are realms of the frame's agent).
