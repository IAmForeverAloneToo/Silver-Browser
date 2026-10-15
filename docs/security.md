# Security

What Silver Browser protects, from whom, where the lines are drawn and
what holds each, the bounds every input is held to, the unsafe code it
runs, how it is fuzzed and attacked, and what it does not defend yet.
The threat model lives here from M33 on (T3.3): each milestone updates
it, and the architecture notes point here for the limits they used to
list. As of M34 (2026-10).

## 1. Assets

- **The user's sessions and data**: cookies, the storage of each site
  (`localStorage`, `sessionStorage`), the HTTP cache, the history, the
  permissions the user gave, and the profile on disk that keeps them.
- **The user's credentials as typed**: what forms send; the browser
  keeps no passwords (no password manager yet).
- **The local network**: devices and services behind the user's
  router or on the machine, which a page on the internet should not
  reach unasked.
- **The machine**: its files, processes and other applications, which
  no page may reach.
- **Confidentiality across sites**: what one site's documents hold,
  which another site's must not read, frame, or infer from what is
  sent to it.

## 2. Adversaries

- **A web page**, and the frames it embeds, of any origin: it runs
  scripts and WebAssembly, makes requests, frames other sites, is
  framed by them, posts messages, and feeds the engine any bytes it
  likes as HTML, CSS, SVG, XML, images, fonts and scripts. The main
  adversary.
- **A network attacker** between the browser and a server: reads and
  changes what goes in clear, and answers for any host it can reach
  the user's DNS for.
- **A page that exploits the engine**: turns a bug in the engine or a
  library under it into code running in its renderer's process (M34),
  which shows its site's documents alone, holds no cookie, storage or
  profile, and on Windows runs sandboxed (section 4.10). It is then a
  *compromised renderer*: it speaks the browser's protocol as it likes,
  and the browser holds every message to what its site may ask.
  Built as one process (`--features single-process`), such a page has
  all of the browser.

Out of scope: a user who installs something hostile on their own
machine, an attacker with the user's account on it, and a hostile
operating system.

## 3. Trust levels

| What | Trusted to | Held by |
|---|---|---|
| The workspace's Rust | be memory-safe and keep its rules | `#![forbid(unsafe_code)]` everywhere but the shell's platform module; the tests, the gate and review |
| Third-party crates | be correct and memory-safe at their safe interfaces | pinned in `Cargo.lock`, `cargo deny` (advisories and licences), the unsafe inventory (section 5) |
| C under the crates (QuickJS-ng, aws-lc) | nothing beyond what fuzzing and their upstreams give | the bounds of section 8 on what reaches them; the process boundary (section 4.10): QuickJS-ng runs in the renderers, aws-lc in the browser |
| A renderer process | nothing: what it says is a compromised renderer's word | the browser's checks on each message against the renderer's site lock and grants, a kill on the first it may not send; the sandbox |
| Page content | nothing | everything below |
| The network | nothing | TLS, HSTS, mixed-content blocking, the egress rule |
| The profile on disk | to be the user's | encryption at rest (section 4.9) |

## 4. Boundaries, and what holds them

### 4.1 Origins and sites

- **Fetch** (M27): a request's mode and credentials decide what its
  caller may read; a no-cors response across origins is opaque to
  scripts and to the canvas; CORS checks the preflight and the
  response; `Cross-Origin-Resource-Policy` refuses a resource to
  another origin or site; a script's error from another origin is
  muted. The adversarial page of M27.7 (section 7) tries 26 reads of
  another origin and learns nothing unless the origin allows it by
  CORS.
- **Cookies** (M27, M33.2): `Secure` cookies only over potentially
  trustworthy URLs and only set from them; `HttpOnly` ones never a
  script's; `SameSite=Strict` ones only same-site, `Lax` ones besides
  on a top-level navigation by a safe method, `None` ones anywhere
  (third-party cookies are not blocked by default, as in the reference
  browser; a setting for it is G93); `Partitioned` cookies (CHIPS)
  kept under the top-level site they were set under.
- **Storage, cache, connections** (M33.2): a frame in a cross-site
  context keeps its storage under its origin and the top-level site;
  the HTTP cache, the connection pools and TLS sessions are keyed by
  the top-level site too, so that a site cannot learn from another's
  cached responses or warm connections.
- **Referrers** (M33.3): `strict-origin-when-cross-origin` by default,
  `Referrer-Policy` and `referrerpolicy` honoured, no credentials or
  fragment ever, the URL cut to its origin past 4,096 bytes.

### 4.2 Frames

- **Each frame is an engine of its own** (M33.2) with its own realm,
  heap (64 MiB, `MAX_FRAME_HEAP`), fetches and raster, drawn into its
  parent as a canvas is, reached only by messages through the tab's
  frame tree. Every frame is cross-origin to every other, the same
  origin included (G90, open after M34): no frame reads anything of
  another that a frame of another origin could not. A frame of another
  site is shown by its site's renderer (section 4.10).
- **Limits**: 100 frames in a tab (`MAX_FRAMES`), 16 deep
  (`MAX_FRAME_DEPTH`); a frame never loads a document an ancestor
  shows.
- **Sandboxing** (M33.2): the `sandbox` attribute's flags and CSP's
  `sandbox` give a frame an opaque origin, no scripts (parsed as with
  scripting off, styled so), no forms, no popups, no top navigation,
  no downloads, as its tokens allow.
- **Being framed**: `X-Frame-Options` and `frame-ancestors` refuse a
  response's document a frame; the frame shows an error page of
  another origin.
- **Messages**: `postMessage` delivers only when the target window's
  origin is the one named, or `*`.

### 4.3 Content Security Policy

Every document's policies, from header fields and `<meta>`, enforced
and report-only (M33.1): fetch directives for every destination,
inline scripts, styles, handlers and `javascript:` URLs by
`'unsafe-inline'`, nonces and hashes (with `'unsafe-hashes'`),
`'strict-dynamic'`, `eval`, `Function` and string timers by
`'unsafe-eval'`, WebAssembly by `'wasm-unsafe-eval'`, `base-uri`,
`form-action`, `frame-ancestors`, `sandbox` and
`upgrade-insecure-requests`; nonces hidden from selectors when a policy
came from a header; violations as `securitypolicyviolation` events,
console lines and `report-uri` reports. Workers and frames have their
own. Trusted Types (G94) and the Reporting API (G95) are not here.

### 4.4 Transport

TLS 1.3 and 1.2 by `rustls` over `aws-lc`, against the root
certificates `webpki-roots` bundles (Mozilla's set); nothing weaker is
offered. HSTS (M33.3): a host that sent the
field over `https:` is fetched over it from then on, its subdomains
too when it said so, kept in the profile, 10,000 hosts at most and a
year at most each, and the reference browser's preload list (95,136
hosts as of 2026-10-13) known from the start.
`Upgrade-Insecure-Requests` is sent on navigations and its
directive honoured; mixed content is upgraded or blocked as the
reference browser does; a form from a secure page to `http:` waits for
the user on a warning page. Revocation and Certificate Transparency
(G100) and Encrypted Client Hello (G101) are not checked or used.

### 4.5 The local network

Local Network Access (M33.4): a request from a public page to the
local network or the machine, or from a local page to the machine,
waits for the user to allow the page's site (a prompt under the
toolbar, sixty seconds at most, the answer kept in the profile); one
from a page that is not a secure context is refused at once; a frame
asks as its top-level site when its origin or its element's `allow`
lets it, a worker as its creator.

### 4.6 The egress rule

Only `net` opens sockets and reads resources by URL (T3.4): the gate
and CI fail when `TcpStream`, `UdpSocket`, `ToSocketAddrs` or `rustls`
appears in another crate or in the fuzz targets. Nothing the browser
holds leaves it but through `net`'s fetch, which holds the rules
above.

### 4.7 Scripts

QuickJS-ng through `rquickjs`, one realm for each document and worker:
a 256 MiB heap at most (`MAX_SCRIPT_HEAP`, 64 MiB in a frame), past
which the realm is stopped and the user told; a 256 KiB stack
(`MAX_SCRIPT_STACK`); and a run stopped after 10,000 interrupt checks
(`MAX_SCRIPT_STEPS`, 2 to 5 seconds of running code). Workers have no
such budget and run on threads of their own. WebAssembly runs in
`wasmi`, an interpreter in Rust: a module of 64 MiB at most, compiled
as its bytes are charged to the run. Scripts reach the document only
through the engine's host calls, never its memory.

Scripts run by default where pages are renderers' (the many-process
build's window, since M34), `--disable-scripts` turning them off; in
the one-process build and in `--print-to-pdf`'s page, which the
browser's own process shows unsandboxed (G124), they run only when
`--enable-scripts` asks (risk R2).

### 4.8 Parsers and decoders

Every parser and decoder is bounded (section 8): nesting depths, node
and byte counts, image sides and pixels, animation frames, font and
glyph caches, canvas sizes, path points. Image decoding runs inside
`catch_unwind`, so that a panicking decoder fails one image. Each is
fuzzed (section 6).

### 4.9 The profile

`redb` files in the directory the shell chooses, written by `profile`
alone. Every value is sealed with AES-256-GCM under a data key, with
the table and the key as associated data (M33); the data key is
wrapped by DPAPI for the user on Windows, so that another user of the
machine, or the files copied off it, read nothing. On macOS and Linux
the key is not wrapped yet (G98). Another program running as the user
can still ask DPAPI, as it can for the reference browser.

### 4.10 The process boundary

M34 (docs/design/processes.md). The browser process keeps the window,
the network, the cookie jar, the storage, the HTTP cache, the profile,
the permissions and the frames' tree; each site's documents are
engines in a *renderer process* of their own, the same executable run
with `--renderer`, which speaks to the browser over its standard input
and output by the protocol of `engine::ipc` (a versioned schema, each
message at most `MAX_MESSAGE`).

- **Site isolation.** A renderer is locked to one site (scheme and
  registrable domain) at its first document: a navigation whose
  response is of another site is handed over at its head to a renderer
  of that site, never read by the one that asked; a frame of another
  site is shown by its site's renderer in a hole of its parent's
  frame, the browser compositing them. At most `MAX_RENDERERS` (16) a
  tab, the least recently shown not showing anything let go first.
- **The browser's word, not the renderer's.** Each message is checked
  against the renderer's lock and the frames the browser granted it,
  and a renderer that sends one it may not is killed: a request
  claiming another origin, initiator, partition or local network
  access; a request of no origin that is no navigation, a navigation's
  mode outside one, a stream that is no navigation's, a forbidden
  method or field; another site's cookies or storage; a frame it does
  not host; a message stamped with another origin or a navigation in
  another's name; a view or a surface it was not given; a malformed,
  truncated, overlong or unknown message. The browser decides the
  partition, the cookies' context and the referrer it sends, filters
  opaque responses by Opaque Response Blocking before a renderer has
  them, keeps another site's download's body, and refuses `file:` to a
  renderer not of files. `service::compromised_tests` is a renderer
  speaking the protocol itself, asking for each.
- **The sandbox on Windows** (D2). A renderer starts with a restricted
  token of the browser's: every privilege but traversal removed, every
  group deny-only but `Users`, `Everyone` and the logon session's, and
  `Users`, `Everyone` and `RESTRICTED` as restricting SIDs (its default
  DACL admitting `RESTRICTED` too, for what it makes for itself); at
  low integrity; suspended, put in a job and resumed. The job kills it
  when the browser's hold closes, admits no second process, limits its
  committed memory to `MAX_RENDERER_MEMORY` (4 GiB) and closes the
  desktop's global state (system parameters, display settings, exiting
  Windows, global atoms, other processes' user handles, the clipboard,
  other desktops). It inherits its three pipes and its crash channel,
  named by a handle list, and nothing else. `--check-processes sandbox`
  runs probes inside one: the user's files and folders, the temporary
  folder, the profile's folder, a folder writable by `Users` (at low
  integrity), the executable, a child process, past its memory and the
  desktop's state are refused; the system's fonts are read.
- **macOS and Linux** run renderers as processes with no sandbox yet
  (G113): a Seatbelt profile by `sandbox_init`, and seccomp-BPF in new
  user, PID and network namespaces, are what each needs.
- **What a compromised renderer still has**: everything of its own
  site's documents in the tab (their cookies through the browser, their
  storage); the network, by a socket of its own on Windows (G111; it
  holds no cookie to send); the reference browser's harder lockdown is
  not here (G112); the URL it reports for its page is shown as it
  says (G123). The browser process itself runs unsandboxed, as the
  reference browser's does.
- **Crashes** (D34). A renderer that dies leaves the browser's own page
  saying so, in the tab or in a frame's hole, until a reload brings it
  back in a new renderer; the browser goes on. On Windows the browser
  writes a minidump of a renderer that crashed (by an exception or a
  panic that ended it) beside the profile, the newest
  `MAX_CRASH_DUMPS` (8) kept; nothing is sent anywhere.

## 5. Unsafe code

**In the workspace** (M33.5): every crate, test, bench, example and
fuzz target says `#![forbid(unsafe_code)]` but the shell, which says
`#![deny(unsafe_code)]` and allows it in its platform module alone,
51 blocks at M34, each with its `// SAFETY:` argument: M33's four
(`SystemParametersInfoW` for the user's reduced motion; DPAPI's call
to wrap or unwrap the profile's key, the slice of its output and
`LocalFree`), and M34's 47 for the renderers on Windows: their pipes
and the handles they inherit (`CreatePipe`, `SetHandleInformation`, the
attribute list of `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`), the job and its
limits, the restricted token (`OpenProcessToken`, `CreateWellKnownSid`,
`GetTokenInformation`, `EqualSid`, `CreateRestrictedToken`, the default
DACL's entries, `SetTokenInformation` for the integrity),
`CreateProcessAsUserW`, `ResumeThread`, `TerminateProcess` and the exit
code; and the crash channel: its mapping and events, the browser's
wait and `MiniDumpWriteDump`, the renderer's view of the block, its
unhandled-exception filter and the check's `RaiseException`. The shell's test
`unsafe_is_written_in_the_platform_module_alone_each_block_argued`
holds every source file to this, and CI greps the same (T3.2).

**Under it**: of the 379 packages `Cargo.lock` holds (every platform's
together), 261 write `unsafe` somewhere and 118 none. Those the
attack surface runs through:

| Crate | Version | What for | Blocks | Fns | Traits, impls | Externs |
|---|---|---|---:|---:|---:|---:|
| `rquickjs-sys` | 0.14.0 | QuickJS-ng, in C | 2 | 30 | 0 | 6,098 |
| `rquickjs-core` | 0.14.0 | its safe interface | 432 | 141 | 60 | 29 |
| `wasmi` | 2.0.0 | WebAssembly | 213 | 59 | 18 | 0 |
| `aws-lc-sys` | 0.45.0 | TLS's cryptography, in C | 8,844 | 204 | 0 | 3,491 |
| `aws-lc-rs` | 1.18.1 | its safe interface | 307 | 8 | 46 | 6 |
| `mio` | 1.2.3 | the sockets' event loop | 105 | 28 | 21 | 0 |
| `flate2` | 1.1.10 | gzip and deflate bodies | 26 | 6 | 2 | 2 |
| `zune-jpeg` | 0.5.15 | JPEG | 56 | 23 | 0 | 0 |
| `tiny-skia` | 0.12.0 | the canvas's raster | 134 | 1 | 16 | 0 |
| `rustybuzz` | 0.20.1 | shaping | 1 | 0 | 14 | 0 |
| `redb` | 4.3.0 | the profile | 30 | 18 | 5 | 1 |
| `hashbrown` | 0.17.1 | hash tables | 322 | 107 | 26 | 0 |
| `winit` | 0.30.13 | windows and input | 732 | 65 | 70 | 56 |
| `softbuffer` | 0.4.8 | presenting pixels | 62 | 11 | 10 | 1 |

`rustls` and `rustls-webpki`, `png` and its inflaters, the font
parsers and the workspace's own parsers write none. The C is what
weighs: QuickJS-ng meets every script a page sends, and aws-lc every
byte of TLS. `cargo run -p engine --example
unsafe_inventory -- Cargo.lock` counts the whole lock file again
(after `cargo fetch`), for each package its `unsafe` blocks,
functions, traits and impls, and extern blocks; a lock-file change
that adds a package writing `unsafe` is read with it.

## 6. Fuzzing

30 libFuzzer targets in `fuzz/` (M33.6, M34), each holding its subject to
not panicking, to its bounds, and to what it must keep (a round trip,
a differential comparison with a fresh engine, a policy never allowing
what it refused):

- *parsers*: `html_parse`, `xml_parse`, `css_stylesheet`,
  `selector_parse`, `media_query`, `values`, `url_parse`,
  `mime_sniff`, `encoding_decode`, `http_response`, `script`, `wasm`,
  `svg`;
- *decoders*: `image_decode`;
- *the engine*: `style_document`, `animations`, `pipeline`, `mutation`,
  `dom_api`, `input`, `canvas`, `clone`, `frames`;
- *the network's rules*: `csp`, `referrer`, `hsts`, `cookies`,
  `storage`, `http_cache`;
- *the process boundary*: `ipc_decode`, any bytes read as the
  messages of either side, frame by frame and whole, each decoded one
  encoded again the same; its seeds include messages recorded from a
  session through renderers (`examples/ipc_seeds.rs`).

Each target's corpus, minimized, is committed under `fuzz/seeds/`;
CI runs every target nightly from its cache and the seeds; each
milestone runs the targets it touched, and its notes record what they
found. OSS-Fuzz's
continuous fuzzing waits for the repository to be public (G99); its
application would hold the targets above, built by `cargo fuzz`, with
the seeds as corpora and the maintainer as contact.

## 7. The adversarial site

`crates/engine/tests/security/` holds pages that attack the browser,
run by the engine's tests against victims served by the test server
(`loader/security_tests.rs`, `loader/adversarial_tests.rs`):

- `cross-origin.html` (M27.7): 26 ways of reading another origin;
  against a victim that allows nothing it learns nothing, against one
  that allows it by CORS it learns by the CORS reads alone.
- `frames.html`, `csp.html`, `cookies.html`, `network.html` (M33.8):
  framing a victim that refuses it, a sandboxed frame's reads, form,
  script, handler, navigation of the top and popup, reads across frames and a
  message meant for another origin; injected markup under a strict
  policy trying every way to run code, style the page, reach the
  attacker and move the page's scripts; cookies carried and read
  across sites, an `HttpOnly` cookie read, a `Secure` one kept in
  clear, a secret path in a `Referer`; a request in clear to a known
  HTTPS-only host, and one from a public page to the local network.
  Each runs with the defences on, where none of its 56 attacks may
  succeed, and with each defence taken away, where each must (the nine
  reads across frames aside, which G90 keeps closed either way).

Every page above runs again in a tab of renderers
(`service/adversarial_tests.rs`, M34.7), the attacker's and the
victim's documents in renderers of their sites, with the same
outcomes. And `service/compromised_tests.rs` is a renderer that speaks
the protocol itself: twenty-one requests it may not make, each its
renderer killed with the browser's word on why and the tab going on;
another site's HTML by no-cors, a `file:` URL and another site's
download answered without what they hold.

## 8. Limits

Every bound the crates keep on what a page can make them do, by the
constant that holds it (generated from the sources with the first
sentence of each constant's documentation; the architecture notes of
earlier milestones say why each is what it is):

| Crate | Constant | Value | What it bounds |
|---|---|---|---|
| `canvas` | `MAX_STROKE_RESOLUTION` | `65_536.0` | The finest a stroke's outline and dashes are traced, as a scale of the user's space: past it a curve under a vast scale is traced as at this one, as the bitmap's single-precision coordinates hold less there anyway. |
| `canvas` | `MAX_CANVAS_PIXELS` | `1 << 25` | The most pixels a canvas's bitmap may have. |
| `canvas` | `MAX_CANVAS_SIDE` | `16_384` | The largest side of a canvas's bitmap. |
| `canvas` | `MAX_CANVAS_STATES` | `1_024` | The most states a context keeps saved. |
| `canvas` | `MAX_PATH_POINTS` | `1 << 20` | The most points a path holds. |
| `canvas` | `MAX_SHADOW_BLUR` | `256.0` | The largest shadow blur drawn. |
| `canvas` | `MAX_DASHES` | `1_024` | The longest dash list. |
| `css` | `MAX_NESTING` | `128` | How deep blocks may nest before a block is kept raw rather than parsed. |
| `dom` | `MAX_DEPTH` | `512` | The deepest elements may nest: 512, the HTML parser's cap on its stack of open elements (`html::MAX_DEPTH` is this constant), which keeps the recursive walks over a finished tree within any thread's stack. |
| `dom` | `MAX_NODES` | `1 << 22` | The most nodes a document may hold alive through its mutation API: about four million, past any real page (the largest have a few hundred thousand) and short of what would exhaust memory before their attributes and text are counted. |
| `dom` | `MAX_JOURNAL` | `1 << 16` | The most records the journal keeps between two drains. |
| `dom` | `MAX_DATA_BYTES` | `256 << 20` | The most bytes of text, tag names and attribute names and values a document may hold in its live nodes through its mutation API: 256 MiB, what a script's heap may hold (M25), so that a script cannot make the document keep a string many times over that it holds once. |
| `dom` | `MAX_GENERATION` | `1 << 21` | The generations a slot may have: a slot is retired once this many nodes have had it, so that an id fits in the 53 bits a script's number holds exactly ([`NodeId::to_bits`]). |
| `engine` | `MAX_CONTROL` | `1 << 24` | The most bytes a text control holds, the document's own limit standing behind it. |
| `engine` | `MAX_FRAMES` | `100` | The most frames a tab has, those of the documents its history keeps included: past them an `iframe` stays empty (the reference browser allows 1,000 a page; a frame here is a whole engine). |
| `engine` | `MAX_FRAME_DEPTH` | `16` | How deep frames nest: a frame this deep makes none of its own. |
| `engine` | `MAX_FRAME_HEAP` | `64 << 20` | The heap a frame's realm may use (M33.2), less than a top-level document's `js::MAX_SCRIPT_HEAP`. |
| `engine` | `MAX_FRAME_TASKS` | `1024` | The most tasks a frame's inbox holds: past them what other frames send it is dropped. |
| `engine` | `MAX_DECODE_THREADS` | `4` | The most images decoded at once, each on a thread of its own. |
| `engine` | `MAX_DECODED_BYTES` | `512 << 20` | The bytes of decoded pixels a document keeps (M23.5): past it the images drawn longest ago, none drawn in the frame, are dropped, and decoded again when next drawn. |
| `engine` | `MAX_ICON_BYTES` | `1 << 20` | The most bytes an icon is kept with: no icon needs more. |
| `engine` | `MAX_FETCH_THREADS` | `32` | The most fetches in flight at once; further ones wait in the order they were made (docs/design/net.md, "Threads, not an asynchronous runtime"). |
| `engine` | `MAX_FRAME_FETCH_THREADS` | `6` | The most threads a frame's engine fetches on (M33.2): a frame is one of up to `MAX_FRAMES` a tab, each with its own. |
| `engine` | `MAX_IMPORT_DEPTH` | `16` | How deep imports go: a sheet's imports are one deeper than it, and one deeper than this is not fetched. |
| `engine` | `MAX_ENTRIES` | `50` | The most entries a history holds: past it, the oldest goes, as in Chromium and Firefox. |
| `engine` | `MAX_KEPT` | `6` | The most documents the back-forward cache keeps besides the one shown, as Chromium's does by default: past it, the one shown least recently goes. |
| `engine` | `MAX_KEPT` | `1000` | The most answers kept: past it, a new answer is not kept. |
| `engine` | `MAX_CANVAS_MEMORY` | `512 << 20` | The most bytes a document's bitmaps hold in all: a canvas made or resized past it has no bitmap, and draws nothing. |
| `engine` | `MAX_CANVAS_OBJECTS` | `1 << 18` | The most `Path2D`s, gradients and patterns a document's scripts hold at once: making one more is a `RangeError`. |
| `engine` | `MAX_CANVAS_POINTS` | `1 << 22` | The most points a document's paths hold together, contexts' and `Path2D`s': past it, a point is not added. |
| `engine` | `MAX_CANVAS_STOPS` | `1 << 20` | The most colour stops a document's gradients hold together: past it, a stop is not added. |
| `engine` | `MAX_DRAWN_DECODED` | `256 << 20` | The most bytes of pixels decoded for drawing a document keeps (M30): past it an image is decoded again for each draw. |
| `engine` | `MAX_KEPT_COLOURS` | `512` | The most colours [`Canvases`] keeps parsed; past it they are let go. |
| `engine` | `MAX_SCRIPT_FETCHES` | `1000` | The most fetches a document has in flight; more are refused. |
| `engine` | `MAX_REQUEST_BODY` | `64 << 20` | The most bytes a script's request body holds. |
| `engine` | `MAX_REQUEST_HEADERS` | `256 << 10` | The most bytes a script's header list holds, names and values. |
| `engine` | `MAX_SCRIPT_NESTING` | `6` | The most scripts that run nested, one inside another's call, the outermost counted (M25): past it no further one runs, and the console says so. |
| `engine` | `MAX_SCRIPT_BYTES` | `32 << 20` | The most bytes an external script may hold; past it its fetch fails, and it is a script that failed (M24.5). |
| `engine` | `MAX_CONSOLE_ENTRIES` | `1_000` | The most entries a document's console keeps, the oldest dropped. |
| `engine` | `MAX_CONSOLE_MESSAGE` | `16 << 10` | The most bytes of an entry's text, cut at a character boundary. |
| `engine` | `MAX_TIMERS` | `100_000` | The most timers a document may have set at once (M26): past them `setTimeout` throws a `RangeError`. |
| `engine` | `MAX_REPORTS` | `100` | The console's level for `violation`: an error for a refusal, a warning for a report. |
| `engine` | `MAX_PORTS` | `65_536` | A document's ports, its workers' included: past it, a new channel is refused. |
| `engine` | `MAX_WORKERS` | `16` | A document's live workers, its workers' own included: past it, a new one waits for one to end, as the standard allows. |
| `engine` | `MAX_WORKER_DEPTH` | `8` | How deep workers may make workers, from the document: a worker made deeper fails to load. |
| `engine` | `MAX_MESSAGE_BYTES` | `256 << 20` | The most a message's serialized form may hold, in bytes. |
| `engine` | `MAX_QUEUED_BYTES` | `512 << 20` | The most the messages waiting in one realm's inbox may hold, in bytes. |
| `engine` | `MAX_WORKER_FETCH_THREADS` | `6` | The threads a worker's fetches run on at once. |
| `engine` | `MAX_ROWS` | `20` | The most rows a drop-down's list shows at once. |
| `engine` | `MAX_MESSAGE` | `16 << 20` | The longest message, in bytes. A body or a picture longer goes in pieces of at most [`MAX_CHUNK`]. |
| `engine` | `MAX_CHUNK` | `1 << 20` | The longest piece of a body in one message. |
| `engine` | `HELLO_TIMEOUT` | `10 s` | How long a renderer has to answer the hello before it is killed. |
| `engine` | `MAX_RENDERER_MEMORY` | `4 << 30` | The memory a renderer's process may commit, where its sandbox can limit it (M34.3, D2): past it, its allocations fail. |
| `engine` | `MAX_RENDERER_FETCHES` | `256` | The most fetches one renderer has in flight; one past it is answered with an error. |
| `engine` | `MAX_HANDOVER_BUFFER` | `16 << 20` | How much of a body waits for the renderer it goes to; past it the network waits. |
| `engine` | `HANDOVER_TIMEOUT` | `30 s` | How long a response waits for its renderer before it is given up. |
| `engine` | `MAX_RENDERERS` | `16` | The most renderers a tab has alive at once, the spare among them. |
| `engine` | `MAX_SURFACE_PIXELS` | `4096 * 4096` | The most raster pixels a view's frame has: one bigger is drawn clipped. |
| `silver-browser` | `MAX_CRASH_DUMPS` | `8` | The most dumps kept: past it, the oldest go. |
| `html` | `MAX_DEPTH` | `dom::MAX_DEPTH` | The most elements that may be open at once. |
| `html` | `MAX_XML_DEPTH` | `512` | The most elements open at once. |
| `html` | `MAX_ENTITY_EXPANSION` | `1 << 20` | The most characters entities may expand to in a document. |
| `html` | `MAX_ENTITY_FACTOR` | `10` | How many times the document's own length its entities may expand to. |
| `html` | `MAX_ENTITY_NESTING` | `32` | How deep entities may refer to entities. |
| `images` | `MAX_IMAGE_BYTES` | `32 << 20` | The largest encoded image taken, in bytes. |
| `images` | `MAX_IMAGE_SIDE` | `16_384` | The largest side of an image, in pixels. |
| `images` | `MAX_IMAGE_PIXELS` | `1 << 25` | The most pixels an image may have: width times height. |
| `images` | `MAX_FRAMES` | `1_000` | The most frames an animation may have. |
| `images` | `MAX_ANIMATION_BYTES` | `256 << 20` | The most bytes an animation's composed frames may take; past it the image keeps its first frame alone. |
| `js` | `MAX_HANDLE` | `(1 << 53) - 1` | The largest number a handle may be: what a script's number holds exactly. |
| `js` | `MAX_SCRIPT_HEAP` | `256 << 20` | The most heap a realm may hold, in bytes. |
| `js` | `MAX_SCRIPT_STEPS` | `10_000` | The interrupt checks a run may take, each about 10,000 of QuickJS-ng's operations: 2 to 5 seconds of running code. |
| `js` | `MAX_SCRIPT_STACK` | `256 << 10` | The native stack a realm's scripts may use, in bytes. |
| `js` | `MAX_ANSWERS` | `1024` | The answers kept, by pattern and value, before they are forgotten. |
| `js` | `MAX_WASM_MODULE_BYTES` | `64 << 20` | The largest module a realm compiles. |
| `js` | `MAX_WASM_MEMORY` | `256 << 20` | What a realm's memories and tables hold, in all. |
| `js` | `MAX_WASM_INSTANCES` | `1_000` | A realm's instances; its memories and tables, ten times as many each. |
| `js` | `MAX_WASM_TABLE_ELEMENTS` | `10_000_000` | A table's length. |
| `js` | `MAX_WASM_NESTING` | `64` | Calls between modules and scripts, nested. |
| `js` | `MAX_WASM_RECURSION` | `10_000` | A module's calls, nested; deeper is a stack overflow (`RangeError`), as the reference browser's is. |
| `layout` | `MAX_SET` | `1024` | The most counters one element's set holds. |
| `layout` | `MAX_COUNTERS` | `1 << 20` | The most counters one walk instantiates. |
| `layout` | `MAX_GENERATED_TEXT` | `1 << 20` | The longest one pseudo-element's text, in bytes: a `::before`'s, an `::after`'s or a marker's. |
| `layout` | `MAX_TEXTS` | `1 << 28` | The most text, in bytes, a walk writes in all. |
| `layout` | `MIN_LINE` | `-10_000` | The first line coordinate the grid may have (section 5.4): line -10,000. |
| `layout` | `MAX_LINE` | `10_000` | The last line coordinate the grid may have (section 5.4). |
| `layout` | `MAX_EXPLICIT_TRACKS` | `MAX_LINE as usize` | The most tracks the explicit grid has in one axis, however many a track list's repetitions ask for. |
| `layout` | `MAX_SPAN` | `(MAX_LINE - MIN_LINE) as i64` | The most tracks a span may cross: the whole limited grid. |
| `layout` | `MAX_PAGES` | `10_000` | The most pages a document is cut into: what is below the last is on none. |
| `net` | `MAX_CACHE_BYTES` | `64 << 20` | The most the cache's bodies hold in all: past it, the least recently used entries go first. |
| `net` | `MAX_CACHE_ENTRY_BYTES` | `8 << 20` | The largest body the cache keeps. |
| `net` | `MAX_CACHE_ENTRIES` | `10_000` | The most entries the cache keeps. |
| `net` | `MAX_COOKIE_SIZE` | `4096` | The most bytes a cookie's name and value may hold together. |
| `net` | `MAX_ATTRIBUTE_SIZE` | `1024` | The most bytes a cookie attribute's value may hold; a longer one is ignored. |
| `net` | `MAX_COOKIE_AGE` | `Duration::from_secs(400 * 24 * 60 * 60)` | The longest a cookie lives: `Expires` and `Max-Age` past it are brought back to it. |
| `net` | `MAX_COOKIES_PER_DOMAIN` | `180` | The most cookies a domain keeps (the reference browser's; the RFC asks for at least 50). |
| `net` | `MAX_COOKIES` | `3000` | The most cookies the jar keeps. |
| `net` | `MAX_POLICIES` | `64` | The most policies a document keeps; one past them refuses everything a check looks at, so that no page escapes its policy by adding more. |
| `net` | `MAX_RECORDED` | `256` | The most violations a sink keeps until its document takes them; past them the newest are dropped. |
| `net` | `MAX_SAMPLE` | `40` | How many characters of what was blocked a violation's sample keeps. |
| `net` | `MAX_PREFLIGHT_AGE` | `Duration::from_secs(7200)` | The longest a preflight's answer is kept, whatever it says: the reference browser's cap. |
| `net` | `MAX_PREFLIGHT_ENTRIES` | `1024` | The most answers the cache keeps; past it, the oldest go first. |
| `net` | `MAX_REDIRECTS` | `20` | How many redirects a fetch follows (Fetch's HTTP-redirect fetch): the twenty-first is an error. |
| `net` | `MAX_HOSTS` | `10_000` | The most hosts a store keeps: past them, the soonest to expire goes. |
| `net` | `MAX_AGE` | `365 * 86_400` | The longest a field makes a host known, in seconds: a year, as the reference browser keeps it. |
| `net` | `MAX_VINT_LENGTH` | `8` | The longest variable-length integer the WebM signature is read by, in bytes. |
| `net` | `MAX_REFERRER` | `4096` | The longest referrer sent whole: past it, the origin alone (section 8.3, step 6). |
| `net` | `MAX_CONNECTIONS_PER_ORIGIN` | `6` | How many connections may be open to one origin (M14.3). |
| `net` | `MAX_TLS_PARTITIONS` | `64` | The most partitions whose TLS sessions are kept apart at once: past them, all are forgotten. |
| `paint` | `MAX_GRADIENT_PIXELS` | `1 << 22` | The most pixels a gradient is rendered with: a larger tile is rendered smaller and stretched over it. |
| `paint` | `MIN_PERIOD` | `0.5` | The period below which a repeating gradient is drawn as its average colour, the pixels being too coarse to show it (section 3.3). |
| `paint` | `MAX_THREADS` | `8` | The most threads a gradient is rendered on, a band of rows each. |
| `render` | `MAX_FRAME_PIXELS` | `8192 * 8192` | The most pixels one frame may have: an 8192-by-8192 frame is 256 MiB of pixels. |
| `render` | `MAX_PARTIAL_SIDE` | `8191` | Past this many pixels across or down, tiny-skia fills without anti-aliasing what it would otherwise anti-alias (its supersampling works in 16-bit coordinates), so that a fill clipped to part of the frame could differ from the same fill whole: a frame that large is always rasterized whole. |
| `render` | `MAX_DAMAGE_RECTS` | `32` | At most this many damaged rectangles are redrawn one by one; more are redrawn as the one rectangle around them all. |
| `render` | `MAX_DIFF_EDITS` | `512` | How many edits apart two lists may be for [`align`] to match their items one by one (M19): past this many, the items between their shared start and end are all taken as changed, which is right if more than it need be, and the cost of looking stays bounded. |
| `render` | `MAX_TURNED` | `64` | The most images turned for `image-orientation: none` or cut to a slice kept at once, and the most bytes they may hold: past either they are all turned and cut again as they are drawn. |
| `render` | `MAX_TURNED_BYTES` | `64 << 20` | The most bytes the images turned or cut for drawing may hold before they are all made again as drawn. |
| `render` | `MAX_SPACED_TILES` | `4096` | The most tiles a pattern with gaps between them is drawn as one by one: past it, the gaps are left out and the tiles drawn as one fill. |
| `style` | `MAX_MIX_DEPTH` | `8` | How deep interpolated transforms may nest inside one another: a transition interrupted during one starts from it. |
| `style` | `MAX_ANIMATIONS` | `65_536` | The most animations and transitions a document runs together: past it a new one does not start. |
| `style` | `MAX_DEPTH` | `32` | The deepest a calculation may nest, parentheses and math functions together: section 10.8's least that a user agent must support, as selectors' and media queries' parentheses are bounded. |
| `style` | `MAX_LENGTH_PX` | `33_554_431.0` | The largest length, in CSS pixels, a declaration can set in either direction. |
| `style` | `MAX_GRADIENT_STOPS` | `256` | The most colour stops a gradient may have (M23.3): one with more is invalid. |
| `style` | `MAX_IMAGE_LAYERS` | `256` | The most layers a box's background may have (M23.3): a list longer than this makes its declaration invalid. |
| `style` | `MAX_REPRESENTATION` | `60` | The longest representation a style writes, in code points; a longer one is its fallback's (Counter Styles 3 section 2: at least 60 must be supported). |
| `style` | `MAX_FALLBACKS` | `8` | How many fallbacks are followed before `decimal` is used: more than any chain of the predefined styles, which is two long. |
| `style` | `MAX_SUBSTITUTION` | `1 << 20` | The most one substitution may weigh: a function's result, or a value with its functions replaced, a megabyte of tokens - some thirty thousand of them, or a string as long. |
| `style` | `MAX_CASCADE_SUBSTITUTION` | `256 << 20` | The most the substitutions of one cascade may weigh together. |
| `style` | `MAX_PROGRESS` | `1e9` | The most an output progress may be away from zero, either way. |
| `style` | `MAX_NESTING` | `32` | How deep conditions in parentheses may nest before what is inside is left unknown. |
| `style` | `MAX_NESTING` | `32` | How deep `:not()`, `:is()`, `:where()` and `of S` may nest. |
| `svg` | `MAX_SVG_ELEMENTS` | `1 << 18` | The most elements a render tree is built from, `<use>`s' instances counted; past it, what was built is drawn. |
| `svg` | `MAX_USE_DEPTH` | `32` | The most `<use>`s within one another, and the longest chain of gradients' templates; a `<use>` past it draws nothing. |
| `svg` | `MAX_GRADIENT_STOPS` | `1_024` | The most stops a gradient takes; the rest are left out. |
| `svg` | `MAX_SVG_LAYERS` | `16` | The most groups drawn into layers of their own, nested; past it, an opacity goes to each child. |
| `svg` | `MAX_SVG_PIXELS` | `1 << 25` | The most pixels a drawing's bitmap has, as a canvas's; one larger is drawn at the largest size within it and stretched. |
| `svg` | `MAX_SVG_LAYER_PIXELS` | `1 << 28` | The most pixels the layers of one drawing take, together; past it, an opacity goes to each child, as past [`MAX_SVG_LAYERS`]. |
| `text` | `MAX_LEN` | `65_536` | The most bytes the address field holds. |
| `text` | `MAX_MASK_PIXELS` | `4096 * 4096` | The most pixels one glyph mask may hold: a glyph drawn at a size where a single letter is 4096 pixels on a side. |
| `text` | `MAX_MEASURES` | `1 << 16` | The most widths a collection keeps: a few megabytes of words. |
| `ui` | `MAX_CHARS` | `1024` | The most characters of a page's title the window shows. |
| `ui` | `MIN_FIELD_W` | `40.0` | The narrowest the field is drawn. |

Besides these, bounds that are no such constant: a counter keeps to
32 bits; animation times are finite seconds, `NaN` zero; an easing's
output is finite and within 10^9 of zero; a permission prompt waits 60
seconds (`PROMPT_TIMEOUT`).

## 9. Known gaps with security weight

- **The renderers' sandbox is Windows's alone** (G113); macOS and
  Linux run renderers unsandboxed, the process boundary all they have.
- **A renderer reaches the network** by a socket of its own (G111), and
  **the reference browser's lockdown** (a token admitting nothing,
  untrusted integrity, a desktop of their own, win32k closed, the
  mitigation policies) is not here (G112).
- **A sandboxed frame shares its site's renderer** (G114), as frames of
  one site do.
- **The URL a renderer reports is shown as it says** (G123): error and
  made pages commit under the URL they were asked for, so a compromised
  renderer can show another site's URL over its own content.
- **`--print-to-pdf` runs its page in the browser's own process**, a
  tool with no window, unsandboxed (G124).
- **A hung renderer** is not noticed, only a dead one (G120).
- **Frames are cross-origin to each other** whatever their origins
  (G90): the closed side, but same-origin pages that script their
  frames do not work.
- **The HSTS preload list is a copy** of the reference browser's as of
  2026-10-13 (`crates/net/data/hsts_preload.txt`, made by
  `crates/net/examples/hsts_preload.rs` from its
  `transport_security_state_static.json`): a host preloaded since is
  not known until it sends the field, and the copy is refreshed by hand
  until an update service (T6.3) can.
- **No revocation or Certificate Transparency** (G100), **no Encrypted
  Client Hello** (G101).
- **Third-party cookies are sent** (`SameSite=None`), as the reference
  browser sends them; blocking them is a setting to come (G93).
- **Trusted Types** (G94), **the Reporting API** (G95), **Permissions
  Policy** past Local Network Access (G96), **the Permissions API**
  (G97), **Subresource Integrity** (G43).
- **The profile's key is not wrapped on macOS and Linux** (G98).
- **OSS-Fuzz** (G99).
- **CSP's edges** found by the web-platform tests and registered at
  M33 (G104): a nonce hidden only when the parser inserted its element; a
  violation's line and column zero and a worker's violation firing no
  event; a report-only violation beside an enforced hash reported as
  enforced; a violation reported for a script `'strict-dynamic'`'s
  hash allows; a string a timer was refused still making a timer.
- **Popups** (`window.open`, `opener`) wait for M37's tabs; until then
  `_blank` navigates the tab itself.
- **Referrer Policy's edges** (G108): `document.referrer` the first
  hop's, a stylesheet's and a worker's own requests' referrers, and the
  mixed form not held when sent into another frame or redirected to
  `http:`.
- **The profile encryption's edges** (G109): pages freed when an older
  profile was sealed keep their values in the clear until written
  again.
