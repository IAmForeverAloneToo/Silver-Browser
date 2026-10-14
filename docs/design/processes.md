# Design note: the multi-process architecture

Roadmap M34. Written before the code, as the record of the decisions,
and corrected to the code when the milestone ends: where they
disagree, the code wins, and section 4 lists the places, marked
*(corrected)*. What ships is described in `docs/architecture.md`
"Current state", and the threat model it changes lives in
`docs/security.md`.

The browser is one process. Its window, the address bar, the network
client with the cookie jar, the HTTP cache, the HSTS hosts and the
permissions the user gave, the profile and the key that opens it,
every document and every frame of every site, their scripts in
QuickJS-ng's C code, their WebAssembly, their image and font decoders:
all share one address space and the user's every right. A bug that
lets a page run code of its own in any parser, decoder or the script
engine reads every site's cookies from the jar, the profile's file,
and the user's own files, and opens sockets to wherever it likes; a
crash anywhere takes the whole browser down. The frames M33 gave the
engine are each an engine of their own, but in the same process: a
frame of another site is kept apart from its parent by checks in the
same address space as both. Earlier notes left to this milestone,
besides its own items: Opaque Response Blocking and the isolation of a
process from the bodies it may not read (M27, `fetch.md`), the
isolation of workers at the seam they cross (M31), frames moved into
processes of their own and their pixels with them (M33, G90 and G91),
a profile a browser process owns (M28), and the sandbox any JIT would
need first (M29, D21).

This milestone splits the browser in two kinds of process. The
*browser process* keeps the window, the user interface, the profile,
the network and everything a site must not reach: the cookie jar, the
cache, the HSTS hosts, Web Storage, the permissions. A *renderer
process* holds the documents of one site: their engines, scripts,
styles, layout and painting. A renderer is locked to its site: every
request, cookie, storage area and frame it asks the browser for is
checked against that lock, and a message it may not send closes it.
Documents of another site go to that site's renderer, top-level ones
and frames alike, a frame of another site drawn by the browser into
the hole its parent leaves for it. Renderers are capped in number and
the idle ones evicted, least recently used first. On Windows a
renderer runs with a restricted token at low integrity in a job
object, and cannot read the user's files or the profile; the other
platforms' sandboxes are documented. A renderer that dies leaves a
page saying so, which a reload brings back in a new one; a crash's
dump is written by the browser and kept on the machine. The `Engine`
type stays what the engine is; the shell's session drives a proxy of
it, which the browser keeps. One process stays a build feature,
tested as the many are. Acceptance: a renderer killed in the middle
of an animation leaves a page saying so, and a reload recovers; a
renderer taken over, speaking the protocol as it likes, reads no
other site's cookies.

The standards and documents are read at the versions current when
this note was written (14 October 2026):

- the HTML Standard (Living Standard): browsing context groups and
  agent clusters (an agent cluster is keyed by a site unless the
  origin asked to be keyed by itself with `Origin-Agent-Cluster`),
  the session history and its traversal, the navigation's "populate a
  history entry's document by fetching", whose response a browser
  may commit in another agent cluster than the one that asked;
- the Fetch Standard (Living Standard of 6 October 2026): the CORS
  check, the cross-origin resource policy check, network partition
  keys, and "should response to request be blocked due to its MIME
  type"; Opaque Response Blocking is not in it yet, and is read from
  its explainer (`annevk/orb`): the opaque-safelisted types (a
  JavaScript MIME type, `text/css`, `image/svg+xml`), the blocklisted
  ones (HTML, JSON and XML MIME types), the never-sniffed ones, and
  its fifteen steps, which say a response is blocked but not what it
  becomes;
- the reference browser's documents of its process model and site
  isolation and of its sandbox's design (Chromium's `main` branch):
  a site is a scheme and a registrable domain; a site instance is a
  site within a browsing context group; on the desktop each renderer
  is locked to one site and never shares it with another, the process
  limit there being soft; frames of a site reuse a process of that
  site; a spare renderer waits, unlocked, for the next navigation; on
  Windows its renderers run with a token that admits nothing but the
  logon SID, at untrusted integrity, in a job object that forbids
  child processes and the desktop's global state, on a desktop of
  their own, with mitigation policies, and its low-box token denies
  them the network;
- Microsoft's documentation of restricted tokens
  (`CreateRestrictedToken`), mandatory integrity control, job objects
  and their limits, `CreateProcessAsUserW` with a restricted version
  of the caller's token, handle inheritance limited by
  `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`, file mappings,
  `DuplicateHandle`, unhandled exception filters and
  `MiniDumpWriteDump`;
- for the platforms documented and not built: macOS's Seatbelt
  profiles (`sandbox_init`), and Linux's seccomp-BPF with user and PID
  namespaces, as the reference browser uses them.

What the documents leave to the browser was put to the reference
browser (Chrome 154.0.8037.99, headless, a throwaway profile) before
this note was written, from one server answering for `a.localhost`,
`b.localhost`, `sub.b.localhost` and `c.localhost` (scratchpad
`m34/`): a page of `a.localhost` with frames of `a.localhost`,
`b.localhost`, `c.localhost`, `sub.b.localhost`, a `srcdoc` frame
and a sandboxed frame of `a.localhost`. Its DevTools list a target
of its own for each frame of `b`, `c` and `sub.b`, and for the
sandboxed one; none for the frame of `a` or the `srcdoc` frame. Its
renderer processes: four with the frames of `b` and `sub.b`, five
with those of `b` and `c`. So a process per site: `sub.b.localhost`
is in `b.localhost`'s process, a frame of the parent's site and a
`srcdoc` frame in the parent's, and a sandboxed frame of the parent's
own site in a process of its own.

## 1. Decisions

### The process model

**Two kinds of process, one executable (M34.1).** The browser process
is the shell as it is: the window, `ui`, the session, the profile,
and now the services below. A renderer is the same executable started
with `--renderer`, which opens no window and reads its orders from the
browser. One executable keeps a version from ever meeting another
(the schema is versioned all the same, below) and the build and the
packaging as they are. What lives where:

| Browser process | Renderer process |
|-----------------|------------------|
| window, `ui`, the session, clipboard, printing's file | engines: documents, frames, workers |
| the profile and its key (DPAPI) | scripts and WebAssembly |
| `net::Client`: connections, TLS, the cache | parsing, style, layout, painting |
| the cookie jar, HSTS, the permissions and their prompt | image, media and font decoding |
| Web Storage (`localStorage`, `sessionStorage`) | `document.cookie` and storage, asked of the browser |
| the frame tree, the session history | a replica of the frame tree |
| the compositor of frames, input routing | its frames' pixels, in shared memory |
| processes: launch, lock, cap, crashes | |

**A renderer for each site of a tab (D13, M34.1).** A *site* is the
reference browser's: the scheme and the registrable domain of the
`psl` crate (D20), or the host itself where there is none (an IP
address, `localhost`); `file:` is one site, and `about:` pages other
than `about:blank` and `about:srcdoc` another. A document goes to the
renderer of its origin's site; an opaque origin's document goes to the
renderer that asked for it (a `data:` URL, a sandboxed frame without
`allow-same-origin`, an error page). There is one browsing context
group, the tab's (the browser has one tab and no popups until M37), so
a site instance is a site: every document of a site in the tab, top
level or framed, goes to the one renderer of that site. A renderer is
*locked* to its site when it is given its first document and for its
life; until then it may ask for nothing.

**Frames by site (M34.1).** A frame whose document is of its parent's
site stays in its parent's renderer as M33 made it, a child engine of
the parent's; so do the initial `about:blank`, `srcdoc` frames and
frames of an opaque origin, which inherit their creator's place. A
frame whose document is of another site is shown by an engine in that
site's renderer, a *frame root* there, and its parent's engine keeps
a remote frame in its place (below). A sandboxed frame of its parent's
site stays in its parent's renderer, where the reference browser gives
it one of its own (G114).

**The cap and eviction (D13).** At most `MAX_RENDERERS`, 16, live at
once (a spare included). When a new site needs a renderer and there are
sixteen, the one least recently shown of those that show nothing (they
keep only parked history, below, or are the spare) is shut down; if
every one shows something (the top and fifteen frames of other sites),
the frame that asked stays empty, with a console line, as a frame past
`MAX_FRAMES` does: the cap is hard, where the reference browser's is
soft, and fails closed (G118). A top-level navigation always gets its
renderer: the one it leaves shows nothing once it commits. Sixteen is
past the sites the pages measured for M33 frame (tens of frames, a few
sites); the memory a renderer costs is measured at step 2 and the cap
set again at step 12 if it must be.

**The spare.** One renderer is started ahead, unlocked, while there is
room under the cap, so that a navigation to a new site does not wait
for a process to start; it is locked by the first document it is
given, and a new spare started after. Tests turn it off, to count.

### Navigation and the session history

**The browser fetches every navigation (D28, M34.1).** A navigation's
request goes to the network service (below) as any other, made by the
engine that navigates as today. The service follows the redirects; at
the final response it decides where the document goes: when the
response would be shown (the engine's `outcome` is a document, an
error status or an unshowable type; a download or a 204 shows
nothing) and its URL's site is not the asking renderer's lock, the
response is *handed over*: the asking engine's fetch ends with
`LoadError::Moved`, and the browser gives the response, its head and
the rest of its body as it comes, to the engine of the right renderer,
which commits it as a navigation of its own (its checks, the framing
refusal among them, run there, in the renderer of the document's own
site, with the ancestors the browser's tree knows). The asking engine,
told its navigation moved, leaves its document as if navigated away:
a top-level engine parks it (below), a frame's parent turns the frame
into a remote one. A navigation the browser starts (the address bar,
a reload of a discarded entry, a traversal to another site's entry)
goes to the URL's site's renderer at once, and is handed over again if
a redirect leaves the site. No request is made twice: a POST's
response is handed over, not fetched again. An error page commits
where the navigation was asked for, with an opaque origin.

**The session history in the browser, in segments (D28).** A
*segment* is a run of the tab's history entries made by one top-level
engine in one renderer, a site's entries that followed one another.
The browser keeps the tab's history as segments, each with its
renderer (none once discarded) and its entries' URLs and titles as the
renderer reported them, and the current entry as a segment and an
index. An engine keeps the entries of its own segment as it does today
(its back-forward cache included), and is told how many entries the
tab has before and after its segment, so that `history.length` counts
the tab's and a `history.go()` past its segment asks the browser:

- a navigation that stays in the renderer adds an entry to the
  segment, and drops the segments after it;
- a navigation handed over to another site starts a new segment after
  the current entry: the engine left is told to drop its entries
  after its current one and to park its document;
- back and forward within a segment are the engine's own traversal;
  across segments, the browser parks the engine shown and unparks the
  target segment's at the target entry, from its back-forward cache if
  it kept the document, else loaded again; a segment whose renderer is
  gone is restored in a new renderer of its site from the URLs and
  titles kept, and its entry loaded;
- a *parked* engine shows nothing: its document is unloaded as when
  it is navigated away from (kept in the engine's back-forward cache
  if it may be), its timers stopped with it, and its renderer counts
  as showing nothing for the cap unless another of its engines shows
  something.

`history.state` and form state are lost when a segment's renderer is
discarded (G117). A frame's navigations are still not entries of the
tab's history (G92).

### IPC

**Messages over pipes, our own codec (D29, M34.2).** Each renderer has
one pipe each way, the renderer's standard input and output, carrying
messages framed by a 32-bit length; nothing else is inherited by it. A
message is a tag and its fields, written and read by a codec of our
own in `engine::ipc` (no serialization crate is in the tree, and
`serde_json` only for tests; a binary format needs no escaping of
bodies and pixels): integers little-endian, strings and byte strings
by length, every enum by a tag the decoder checks, every length
bounded before anything is allocated (`MAX_MESSAGE`, 16 MiB, a
response body coming in pieces of at most 1 MiB). The decoder takes
any bytes and answers a message or an error, never a panic: it is a
fuzz target (`ipc_decode`). The first message each way is a hello
with `IPC_VERSION`, the schema's version, which a mismatch refuses;
both sides are the same build, so the check guards the schema's
discipline more than a real mismatch: a message's layout never
changes without the version.

**Capability-scoped (D29, M34.2).** There is no message a renderer
can send that the browser carries out on trust. The renderer-to-browser
messages are the interfaces a renderer needs and no more: fetches and
their cancellation, its frames' cookies and storage, writes of its
frames to the tree, messages to other frames, its frames' pixels and
state, input it routes to a frame it embeds, and the answers to what
the browser asked. Every handle in them is checked against what the
browser gave that renderer: a frame must be one it hosts, a surface
one it was granted, a fetch one it began, an origin of its site or
opaque. A message that fails a check is a *bad message*: the browser
logs it, kills the renderer and shows the page saying so, as the
reference browser does. The browser-to-renderer messages are orders
(navigate, show, park, resize, input) and answers.

**Synchronous where a script waits, never the browser (D29).** A
script's `document.cookie`, `localStorage.getItem` or `frames.length`
answers at once, so the renderer's thread asks the browser and waits;
the browser answers those on the renderer's reader thread from the
services (which are behind locks, not on the window's thread), never
waiting on any renderer itself, so no cycle of waits can form and a
busy window does not stall a page. The browser never waits on a
renderer: everything it sends is answered later, if at all, and what
the session asks of the page now (its title, its scroll bar, the
cursor) is read from what the renderer last reported. No renderer
waits on another.

**Threads.** In each process a reader thread per pipe decodes and
routes: answers to their callers, fetch results to their fetches,
the rest to the main thread's queue with a wake (in the browser,
winit's proxy, as the network's wake is today). Writes go through a
lock on the pipe. The renderer's main thread runs its engines as the
shell's loop runs one today, waiting on its queue until the next task
or frame is due.

**Renderers as threads in tests (D30, M34.6).** The protocol runs over
any pair of byte streams, and a renderer is a function, `renderer::run`,
given its streams, its surfaces and its fonts. The shell gives it a
process's pipes; the engine's tests give it a thread and an in-memory
pipe, so that the whole architecture (the services, the locks, the
hand-over, the history, the frames, the compositor) is tested in
`cargo test` without a process, and the shell's tests start real ones
for what only a process has: launching, the sandbox, a crash, a kill.

### The network and storage services

**The network service (D31, M34.1).** The browser's `net::Client`, with
the jar, the cache, the HSTS hosts and the Local Network Access gate,
serves every renderer: the engine's `Network` fetches through a
`Fetch` trait, `net::Client` in one process and a remote fetcher in a
renderer, which sends the request and gets the head, the body in
pieces and the end back, a cancellation sent when the request's abort
handle fires. The service does not take a renderer's word for what
decides security. For each request it checks the frame named is one
the renderer hosts, and the request's origin and initiator of the
renderer's site or opaque; it computes from its own frame tree the
partition (the top-level site and whether the frame is cross-site to
it) and the site for cookies, in place of the renderer's; it strips a
referrer of another site; it serves `file:` URLs only to the `file:`
renderer. CORS, the cross-origin resource policy, HSTS, the cookies
sent and kept, the cache and the gate's prompt are all `net::Client`'s,
so in the browser. What goes back is filtered: no `Set-Cookie` ever
(a renderer never sees the jar), and the body of an opaque response
only if Opaque Response Blocking lets it through.

**Opaque Response Blocking (D31).** For a `no-cors` response of another
origin, the service runs the explainer's steps on the head and the
first 1,024 bytes: a safelisted type passes; a never-sniffed type is
blocked; a blocklisted type with `nosniff`, or on a 206, is blocked;
audio and video by their signatures pass as the media request state
allows; an image by its signature passes; with `nosniff`, a status not
ok, or an `image/`, `audio/` or `video/` type that did not sniff as
one, it is blocked; a type that does not parse passes. Step 14, a body
that parses as JavaScript but not JSON, is done as the reference
browser does it in its first version, by sniffing for what is not
script: an HTML or XML start, a JSON value (a recognizer of our own,
no parser of JavaScript in the browser process) or a JSON parser
breaker (`)]}'`, `{} &&`, `for(;;);`) blocks, anything else passes. A
blocked response reaches the renderer with its status and an empty
body and no headers but `Content-Type`, as the reference browser gives
it, so a page's image or script fails as it would on an empty body.

**Cookies and storage (D31, M34.1).** `document.cookie` asks the
browser with the frame and the document's URL; the browser checks the
frame is the renderer's, the URL of its site, the frame's committed
origin of that site, and answers from the jar with the site for
cookies and the partition of its own tree. Web Storage is the
browser's: each area by its storage key (the origin and, for a
frame of another site than the top, the top-level site, as M33 keyed
it), the key's origin checked against the lock and its top-level site
computed by the browser; `getItem`, `setItem`, `removeItem`, `clear`,
`key` and `length` are asked and answered, the quota kept by the
browser, and a change told to the documents of the same key in every
renderer as a `storage` event. `sessionStorage` is the tab's, kept by
the browser. The profile is written by the browser alone, on its
clock, as the engine writes it today.

### Pixels

**Shared memory, and the browser never draws a page (D32, M34.5).** A
renderer draws each of its shown engines (the top or a frame root) in
software and writes the pixels into a *surface*: memory shared with the
browser, made by the browser at the size the renderer asks (at most
`MAX_SURFACE_PIXELS`, 4096 by 4096; a frame bigger is drawn clipped),
mapped for writing in the renderer and for reading in the browser, two
buffers so that one is written while the other is read. A frame's
message names the buffer, the size, and the state the session reads
(below); the browser copies the pixels out, then tells the renderer the
buffer is free; a renderer that has two buffers in use waits for one
before it draws again, and a frame drawn meanwhile replaces the
waiting one. On Windows a surface is a file mapping duplicated into
the renderer; elsewhere, until a platform has its own, the pixels go in
the message itself. Renderers draw in software: the GPU's driver is a
large surface to give a sandboxed process, and the reference browser
keeps it in a process of its own, which this milestone does not make
(G110). The single process keeps the GPU backend as it is.

**Frames of another site composited by the browser (D32).** The
parent's engine draws an `iframe` whose frame is remote as a *hole*:
its content box cleared to transparent, clipped as the box is, with
anything the page draws over it drawn over it. With each frame the
renderer reports where each of its remote frames is, in its raster
pixels after scrolling and clipping. The browser composites a frame
root's pixels under its parent's at that place, then the parent's over
them, the deepest first, so the page's content over a frame stays over
it and the frame shows only through its hole. The parent never sees a
pixel of the frame, which it could otherwise read for another site. A
remote frame under a transform, an opacity, a filter or a clip that is
not a rectangle is composited as an untransformed rectangle (G116).

**The session's proxy (M34.5).** The `Engine` the shell's session
drives becomes, in a renderer build, a proxy the browser keeps: a
`RemoteTab` with the methods the session calls, whose orders (input,
navigation, resizing, the preferences) are messages and whose answers
(the title, the URL, the load state, whether it can go back, the
scroll bar, the cursor, the link under the pointer, the caret's area,
whether a paste is possible, the console, the notices, the
composited frame) are what the renderers last reported. What the
session asks and waits for in one process comes back as an event
here: a copy's text, a print's PDF (which the browser writes, a
renderer being unable to), focus leaving the page by the keyboard.
The permission prompt is the browser's own, the gate being there.

### Frames across processes

**The frame tree in the browser (D33, M34.1).** The tab's tree of
frames moves into the browser, which holds it and its inboxes'
routing; each renderer keeps a replica. A renderer reads its replica
(a script's `length`, `parent`, `top`, `frames[i]`, a name) and writes
through the browser (a frame added, a frame's children, its document's
origin and sandboxing, its name), waiting for the answer, which brings
the tree as it now is; the browser checks the writer hosts the frame
written (a parent for its children, a frame for its own name and
document) and that every origin written is of the writer's site or
opaque, and sends the new tree to the tab's other renderers. A frame's
number is the browser's to give, so that numbers are unique across
renderers. What M33 put in a frame's inbox (a message, a navigation of
another frame) goes through the browser to the renderer of the target,
the browser checking the sender hosts the frame it sends from, that
HTML's rule allows the navigation (its tree has what the rule reads),
and stamping a message's source and origin from its own tree, not the
sender's word. Message ports go through the browser between renderers,
which keeps where each port is; within a renderer they stay in it.

**A remote frame (D33).** In the parent's engine, an `iframe`'s frame
holds either a child engine, as now, or a remote frame: the frame's
number and what its renderer last reported of it (loaded, settled, its
cursor, its caret, the link under the pointer). The parent sizes it
from its box as it sizes a child engine (the size and scale go to the
frame root through the browser), tells its element of its document's
load when the frame root reports it, and draws its hole. A frame,
once out of its parent's renderer, stays remote for the parent wherever
its later documents go, even back to the parent's own renderer, where
it is a frame root like any other. Same-origin frames remain
cross-origin to each other (G90), as M33 left them.

**Input (D33).** The browser sends the pointer and the wheel to the top
renderer. An engine whose hit test lands on a remote frame answers
with the event's number, the frame and the point in the frame's
pixels; the browser checks the point is within the place that frame
was last reported at, and sends the event on to the frame's renderer,
which may route it deeper. A drag that began in a frame stays with it
until the button is released. A frame root that does not scroll for a
wheel says so, and the browser gives the wheel to its parent as if it
were outside the frame. Keys go to the renderer of the focused frame,
which the browser keeps: a click in a frame focuses it, and sequential
navigation enters and leaves a remote frame by an order and an answer
through the browser. A parent taken over can still keep for itself an
event that fell on its frame, or give its frame one that fell on the
page over it, within the frame's place, where the reference browser
hit-tests in its browser process (G115).

### The sandbox

**A restricted token, low integrity and a job object (D2, M34.3).** On
Windows the browser starts a renderer with `CreateProcessAsUserW`,
suspended, with a restricted version of its own token: every privilege
removed but `SeChangeNotifyPrivilege`; every group made deny-only but
the logon SID, `Everyone` and `Users`; and restricting SIDs of `Users`,
`Everyone`, `RESTRICTED` and the logon SID, so that an access must be
granted to one of those as well as to the user. Files only the user
may open, the profile's folder, the user's documents and the temporary
folder among them, are closed to it; what `Users` may read (the system
folders, the fonts) is open. The token's integrity is low, so it can
write nothing labelled above. The process is assigned to a job object
before it runs a single instruction, then resumed. The job kills its
processes when the browser's handle to it closes; admits one active
process (no child); limits a process's committed memory
(`MAX_RENDERER_MEMORY`, 4 GiB); and forbids the desktop's global state:
`SystemParametersInfo` changes, display settings, exiting Windows,
the global atoms, handles of user objects outside the job, and reading
or writing the clipboard. Only the two pipe handles (and the standard
error, for its log) are inherited, named by
`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`. This is the reference browser's
"limited" token, the one its GPU process ran with; its renderers'
token admits nothing at all, after a start under a second token that
the process drops itself, at untrusted integrity, on a desktop of
their own, with mitigation policies, and a low-box token that denies
the network (G111, G112): a renderer taken over here can still open a
socket of its own, though with no cookie to send, the jar being the
browser's. The tests of the sandbox are a renderer's probes run inside
it: the profile, a file of the user's temporary folder and the
browser's executable opened for writing must be refused; a child
process must be refused; the system's fonts must open.

**The other platforms, documented (M34.3).** On macOS and Linux a
renderer runs as on Windows without the sandbox, its pixels in its
messages, and `docs/security.md` says so and what each needs: on
macOS a Seatbelt profile applied by `sandbox_init` at the renderer's
start, denying file reads but the executable's, the system's
libraries and fonts, and every network and process call; on Linux a
seccomp-BPF filter of the system calls a renderer needs, inside new
user, PID and network namespaces, as the reference browser's are
(G113).

### Crashes

**A renderer's death is a page, not the browser's (D34, M34.4).** The
browser sees a renderer go when its pipe closes or its process ends:
every engine it showed shows the browser's page saying the page's
renderer stopped and that a reload brings it back, a reload starting
a new renderer of the site and restoring the segment (its history's
URLs and titles); frames it showed under other sites' pages show the
same page in their holes, and their parents' `load` is not held by
them. A renderer killed, or one that ends on a bad message, is the
same. The browser goes on.

**Dumps written by the browser, kept on the machine (D34, M34.4).** A
renderer installs an unhandled exception filter and a panic hook,
both in the shell's platform module. On a crash it fills a small block
of memory shared with the browser (its thread, the exception's address
in its own memory, or a panic's message and place), signals an event
the browser waits on, and waits; the browser writes a minidump of it
with `MiniDumpWriteDump`, the exception pointers read in the
renderer's memory, and signals back; the renderer ends. Dumps go to
the `Crashes` folder beside the profile, with a text file of the
panic's message where there is one; at most `MAX_CRASH_DUMPS`, 8, are
kept, the oldest removed. Nothing is sent anywhere: the page saying
the renderer stopped names the file, for the user to share if they
choose. A kill or an abort leaves no dump (nothing runs to write
one).

### One process, and where the code lives

**One process as a build feature (D30, M34.6).** The shell builds the
many processes by default; `--features single-process` builds the
browser as it is today, the session driving an `Engine`, no renderer
started. Both are built, linted and tested by the gate and by CI (the
test job runs the shell's tests once with the feature). The engine
compiles the same either way: its seams (`Fetch`, the storage and
cookie backends, the tree's writes, the remote frame) have a local side
used in one process and in the engine's own tests.

**Where the code lives (D30).** In the engine, which forbids `unsafe`:
`engine::ipc` (the codec, the messages, the channel), `engine::service`
(the browser's side: the network and storage services, the tree, the
processes and their cap, the compositor, `RemoteTab`), and
`engine::renderer` (the renderer's side: its engines, its replica, its
fetcher and backends, its drawing). In the shell's `platform` module,
the workspace's one place for `unsafe`: the processes' launch with the
token, the job and the handle list; the surfaces' file mappings; the
crash events, the filter and the dumps. Each `unsafe` block keeps its
`// SAFETY:` argument above it. A surface is reached only by copying
into and out of it (`copy_in`, `copy_out`), never as a slice, since
another process writes it: a renderer that writes while the browser
reads tears only its own picture, its bytes being bytes. The
`windows-sys` features grow by those these calls need.

**Origins numbered apart.** `net::Origin::new_opaque` numbers opaque
origins from a counter of the process; two renderers would give the
same numbers to different origins, which the browser compares. Each
process's numbers start from its own space, given at its hello.

### Limits, tests and measures

The limits are `MAX_RENDERERS` (16), `MAX_MESSAGE` (16 MiB),
`MAX_SURFACE_PIXELS` (4096 by 4096), `MAX_RENDERER_MEMORY` (4 GiB),
`MAX_CRASH_DUMPS` (8), and a hello not answered in 10 seconds kills the
renderer. They go in `docs/security.md`'s table of limits.

Unit tests in the engine for the codec (every message there and back,
every bound, every truncation), the channel, ORB's steps, each
service's checks with a renderer's word against the browser's, the
hand-over, the history's segments, the tree's writes and routing, the
compositor and the holes, the cap and the spare; equivalence tests,
the same pages drawn by one engine and by a tab of renderers (threads)
giving the same pixels, frames of other sites among them; the
engine's tests of frames, storage, cookies, partitions and security
run again through renderers where they cross processes. The shell's
tests start the real executable: the hello, the sandbox's probes, a
crash's dump, a kill's page and the reload. A compromised renderer is
a test that speaks the protocol itself and asks for what it may not
(below). Benchmarks (`benches/processes.rs`): a renderer's start, a
message's round trip, a fetch through the service, a frame through a
surface, a navigation handed over. T4.3 for what the seams cost the
single process. The fuzz target `ipc_decode`.

**The compromised renderer (M34.7).** A test renderer locked to one
site, speaking the protocol directly, asks: another site's cookies by
`document.cookie` and by a request claiming that site's origin;
another site's storage; a frame it does not host, written or read; a
message stamped with another origin; a navigation of a frame it may
not navigate; a request in another partition; another site's HTML
by `no-cors`; a `file:` URL; a surface it was not granted; a message
past the size limit, a truncated one, an unknown tag. Each is refused,
the renderer is killed where the protocol says so, and the browser
goes on. The adversarial site's pages (M27.7, M33.17) run again in a
tab of renderers.

## 2. Not in this milestone, said rather than hidden

Each item that no milestone holds is registered on the G list at the
documents step; the number follows each.

- *Same-origin scripting across frames*: frames of one origin stay
  cross-origin to each other, now that they may share a renderer;
  QuickJS-ng realms in one runtime are what it needs. G90, open.
- *Frames' pixels on the GPU*: a GPU process, the renderers' drawing on
  it and the compositor with it. Renderers draw in software. G91 and
  G110.
- *The network denied to a renderer*: a low-box (AppContainer) token
  without the `internetClient` capability. G111.
- *The reference browser's lockdown*: a token admitting nothing after
  a start under another, untrusted integrity, a desktop of the
  renderers' own, win32k's system calls closed, and the mitigation
  policies (dynamic code, signed images, extension points). G112.
- *macOS's and Linux's sandboxes*, and shared memory for their
  surfaces. G113.
- *A sandboxed frame in a process of its own*, as the reference
  browser isolates them. G114.
- *Input hit-tested by the browser*: a frame's events are routed by
  its parent's renderer, checked against the frame's place. G115.
- *A remote frame transformed, faded, filtered or clipped other than by
  a rectangle*, composited as a plain rectangle. G116.
- *`history.state` and form state kept by the browser*, lost with a
  discarded segment's renderer. G117.
- *A soft cap*: past `MAX_RENDERERS`, a frame of a new site stays
  empty. G118.
- *Cross-origin isolation*: `Cross-Origin-Opener-Policy`,
  `Cross-Origin-Embedder-Policy`, `crossOriginIsolated`, and
  `Origin-Agent-Cluster`'s processes by origin. G119.
- *A hung renderer*, noticed and offered to be stopped; only a dead
  one is noticed. G120.
- *ORB's JavaScript parse*, done by sniffing for what is not script.
  G121.
- *Workers in processes of their own*: a dedicated worker stays in its
  document's renderer, as in the reference browser; shared and service
  workers do not exist yet.
- *Popups and tabs*, each a browsing context group: with M37's tabs.

## 3. Order of work

Each step a vertical slice, with its tests, a sabotage pass and the
gate.

1. *The IPC codec and channel*: `engine::ipc`, the messages known so
   far, the bounds, the hello and its version, the channel over two
   byte streams with its reader thread and synchronous calls, the
   in-memory pipe, the fuzz target `ipc_decode`.
2. *Renderer processes*: `--renderer`, `renderer::run`'s loop, the
   launch from the browser with the pipes and the handle list, the job
   object that kills on close, the hello and its timeout, exit and
   kill noticed; opaque origins numbered apart; a renderer's start and
   memory measured.
3. *The network service*: the `Fetch` seam in `Network`, the remote
   fetcher, the service's checks and the partition and site for
   cookies from the browser, `Set-Cookie` never sent, `file:` to the
   `file:` renderer, ORB; `document.cookie` through the browser.
4. *The storage service*: Web Storage in the browser, keys checked,
   quotas, `storage` events across renderers, `sessionStorage`, the
   profile written by the browser.
5. *The remote tab*: `RemoteTab` and the session over it, a renderer
   showing the tab's top-level engine, surfaces in shared memory,
   input and state, copy, paste and print, the console and notices,
   behind `--multi-process` until step 10.
6. *Sites and the session history*: the lock at the first document, the
   hand-over of a navigation to another site, segments, parking,
   traversals across segments and their restoring, `history.length`
   and `go()`, the cap, eviction and the spare.
7. *Frames across processes*: the tree in the browser with replicas and
   checked writes, frame numbers, inboxes and ports through the
   browser, remote frames and their holes, the compositor, sizes,
   loads, input routing, focus and keys.
8. *The sandbox*: the restricted token, low integrity, the job's
   limits, the probes.
9. *Crashes*: the page saying a renderer stopped, the reload, bad
   messages, the dumps.
10. *One process as a build feature*: many processes by default,
    `single-process`, the gate and CI both ways.
11. *The compromised renderer* and the adversarial site through
    renderers.
12. *Acceptance*: a renderer killed in an animation,
    `benches/processes.rs`, the cap measured, T4.3, the fuzz runs.
13. *`docs/security.md`*: the process boundary in the threat model, the
    sandbox per platform, the limits.
14. *The documents*.

## 4. Corrected to the code

Where the milestone's code came out otherwise than the sections above
planned, *(corrected)*, by section. The step notes say how each was
found.

*IPC.* *(corrected)* Messages are tagged by a `u16`, unit enums by a
`u8`; a count or a length is held to the bytes left before anything
is allocated for it (step 1).

*The network and storage services.* *(corrected)* Storage events stay
among one renderer's engines, as they stayed among one engine's
documents in one process (step 4). Frames ask for local network access
in their top-level origin's name, which the lock check admits (step 7).
The compromised renderer of step 11 found what the note did not list:
the fetch treats a request of no origin, or in a navigation's mode, as
the user's own load and filters nothing, so the browser now kills a
renderer for a request of no origin that is not a navigation, for a
navigation's mode outside a document's or a frame's navigation, for a
stream that is not a navigation's or a navigation not streamed (the
hand-over is on the streamed path), and for a forbidden method or
field; and another site's response to a navigation that makes no
document (a download) reaches the renderer as its head alone.

*Navigation and the session history.* *(corrected)* Scripts have no
history object yet, so the segments are the browser's alone and no
engine is told how many entries lie before and after its own (step 6).
An error page commits where its navigation was asked for, under the
URL it was asked for, as planned; so does a page the browser makes (the
mixed form's warning), which leaves the browser unable to tell them
from a renderer's lie about its URL (G123, step 11).

*Pixels.* *(corrected)* A view's frames go through the pipe as the
rows that changed, in pieces under `MAX_CHUNK`, on every platform: the
shared surfaces of D32 are not made (G125). A renderer draws at most
one frame each 16 ms and only once the browser took the last. A remote
frame's hole is drawn in the colour `HOLE` (rgb(1, 254, 3), opaque),
and the browser composites a frame root's pixels where its parent's
are that colour, so the parent's content over the frame stays over it;
a hole under a transform, an opacity or a filter is shown as G116
says (step 7).

*Frames across processes.* *(corrected)* No renderer keeps a replica of
the tree: the browser numbers frames, knows their renderers, parents,
origins, URLs, sandboxing and the views that show them, and carries a
message or a navigation posted to a frame of another renderer to it,
checking the sender's frame and origin; a remote frame's window
answers `postMessage` and `location` alone, its `length`, `frames` and
document not read across renderers, nor ports carried between them
(G122). The browser hit-tests the pointer itself, by the holes and the
`HOLE` pixel under it, with capture while a button is held, so G115 is
not needed; the wheel goes to the frame under the pointer and is not
given back to its parent when the frame does not scroll, keys go to the
view last clicked, and sequential focus does not enter a remote frame
(G122). A frame's new view is sized from the hole already drawn for it
(step 9).

*The sandbox.* *(corrected)* The logon session's SID is not a
restricting SID: nothing a renderer does needs it, and it would admit
the session's desktop and named objects; it stays enabled for the
normal check, which user32's start needs. The token's default DACL
admits `RESTRICTED`, without which what the process makes for itself
grants the user alone and the system's libraries fail to start
(0xC0000142). The probes refuse a folder of `Users` written at low
integrity (ProgramData) besides those planned, and the desktop's state
as the job reports it; the network is not probed (a socket in the
shell would break the egress rule's check), and is open (G111). A probe
that makes a file makes it delete-on-close (step 8).

*Crashes.* *(corrected)* The engine catches the panics of image
decoders, of the WebAssembly glue and of the profile, so the panic
hook only keeps the message (with a backtrace); the report is made when
a panic leaves the renderer's main loop, which runs under
`catch_unwind`. A panic on another thread of a renderer is not
reported. `--temporary-profile` keeps no dumps; a renderer that lives
10 seconds after its dump is ended (step 9).

*One process as a build feature.* *(corrected)* `--multi-process` is
gone; `--software` belongs to the one-process build, renderers drawing
in software; `--print-to-pdf` runs its page in the browser's own
process, unsandboxed (G124, step 10).

*Limits, tests and measures.* *(corrected)* The cap measured at step 12
found each renderer holding 120 MB of its own, the CJK fallback fonts
read into every process: a fallback is now read when a text of its
script first needs it, shared by the scripts of its face, and a
renderer at the cap holds 5 MB. A renderer process answers its hello
in 21 to 23 ms. A click reaches the frame that shows it in 25 ms
through renderers against 1.5 ms in one engine, the frame pacing and
the rows through the pipe (M35).

*Registered* (G110 to G125): the GPU (G110); the network (G111); the
lockdown (G112); macOS's and Linux's sandboxes and dumps (G113); a
sandboxed frame's process (G114); G115 not needed; transformed frames
(G116); `history.state` and form state (G117); the soft cap (G118);
cross-origin isolation (G119); a hung renderer (G120); ORB's script
parse (G121); the frame tree's reads and ports across renderers, the
wheel's chaining and focus traversal into frames (G122); the URL a
renderer reports (G123); `--print-to-pdf` in the browser's process
(G124); shared surfaces (G125).
