# Design note: extensions (1Password and uBlock Origin first)

Status: proposed, 2026-11-18, for the lead's approval before any code.
Written against `docs/design/same-origin-frames.md` (G90): its agents,
realms and landing rules are what content scripts run in.

The beta's must-haves are the user's two extensions, 1Password and
uBlock Origin (password filling and ad blocking are beta blockers).
The roadmap's "no extension ecosystem before T4" is overtaken by that:
this note builds the runtime those two need, shaped so that more of
Chrome's extensions can follow without a second design.

## 0. What the two need

Read from their packages before building (phase 0 below makes this
exact, by the manifests and a static scan of the API calls); what each
is known to use:

- **1Password** (Chrome's MV3 build): content scripts in every frame
  (`all_frames`, `match_about_blank`) that find fields and fill them;
  a background service worker; `runtime` and `tabs` messaging, by
  frame; `storage`; `scripting`; `contextMenus`, `notifications`,
  `alarms`, `idle`; the toolbar action and its popup; and
  `nativeMessaging` to the desktop app (optional: the extension signs
  in on its own without it). Its crypto runs in WebAssembly.
- **uBlock Origin**: the full uBO blocks with `webRequest`'s blocking
  form (Manifest V2, as Firefox and uBO's own Chromium MV2 build keep
  it), its filter engine in its background page, partly WebAssembly;
  uBO Lite (MV3) blocks with `declarativeNetRequest` rules instead.
  Both hide elements by cosmetic filters: CSS inserted into pages
  (`tabs.insertCSS` / `scripting.insertCSS`) and content scripts for
  procedural filters and scriptlets (scriptlets injected into the
  page's own world, `world: "MAIN"`).

## 1. Decisions

### The Chrome door

**Packages, identities and APIs as Chrome's, so that Chrome's
extensions remain installable later.** An extension is installed from
a CRX3 file (its signature checked, its ID derived from its public key
as Chrome derives it: the first 128 bits of the key's SHA-256, as
`a`-`p` letters) or from an unpacked directory (developer mode); its
pages are of origin `chrome-extension://<id>`; its API namespace is
`chrome.*`, with `browser.*` as an alias returning promises (Firefox's
form, which uBO and 1Password both accept); Manifest V3 is the model,
and V2's background page and blocking `webRequest` are kept beside it
(below). Nothing in this design is specific to the two extensions:
what they need is built first, by phases, and an extension asking for
an API not built is refused at install with the list of what is
missing, never half-run. The Chrome Web Store itself (its install
button, its update service) is left for later: a CRX downloaded from
it installs, and `update_url` polling is a later phase.

### Where an extension runs

- **Its own origin, its own renderer.** `chrome-extension://<id>` is a
  site of its own (M34's placement by site): the background (an MV3
  service worker, or an MV2 background page, a document with no view),
  the popup, the options page and any extension page are documents or
  workers of that origin, in that renderer. The renderer is sandboxed
  as a page's is; its privileges are the browser's, granted per call.
- **Content scripts in the page's renderer**, as Chrome runs them: an
  isolated world is a **realm in the page's agent runtime** (G90): it
  shares the document (its natives are bound to the frame's engine)
  but not the page's globals or wrappers. A node has one wrapper per
  world: the page's realm and the content script's each keep their own
  map from a node to its object, as realms already do, so `expando`
  properties a page sets are not seen by the extension, and the reverse,
  as in Chrome. Each extension with a content script in a frame gets
  one realm there, made at the first script it injects.
- **`world: "MAIN"` scripts** (uBO's scriptlets) are run in the page's
  own realm, at `document_start`, before the page's first script.

### Injection, by the frame landing rules

- Matched against the frame's URL by the manifest's `matches`,
  `exclude_matches`, `include_globs` and `exclude_globs`; the
  extension's host permissions limit what may match.
- `all_frames: false` injects into the top frame only; `true` into
  every frame, each a realm in that frame's agent.
- `match_about_blank` / `match_origin_as_fallback`: a frame of
  `about:blank`, `about:srcdoc`, `blob:` or `data:` is matched by its
  creator's URL (`about:blank`, `srcdoc`) or its origin (`blob:`), by
  the G90 note's landing rules; the initial `about:blank` made
  synchronously gets its content scripts when it is made, before the
  creator's script continues, as Chrome's do (a field-filler's frame
  made by script and filled at once).
- `run_at`: `document_start` before the page's first script (after the
  document element exists), `document_end` after parsing and before
  `DOMContentLoaded`'s listeners, `document_idle` (the default) after
  `DOMContentLoaded` and before `load`'s, or at `load` at the latest.
- `scripting.executeScript` / `insertCSS` inject at once, into the
  frames asked for (by `frameIds`, or all).

### Messaging

`runtime.sendMessage` / `onMessage`, `tabs.sendMessage` (to a tab, or
one frame by `frameId`), `runtime.connect` / ports, and
`runtime.onMessageExternal` (refused at first). A message crosses
processes through the browser, as M34's frame messages do: the
browser stamps the sender (extension ID, tab, frame number, frame URL
and origin) from its own frame tree, never the sender's word, and
routes it to the extension's renderer or the frame's. `frameId` is the
frame tree's frame number (the top's 0, as Chrome's). Within one
renderer (two frames of one page) a message still goes through the
browser, so that what is stamped is always the browser's.

### The network: blocking and rules

All of a tab's requests already pass through the browser's network
service (M34). There, before a request is sent:

- **`declarativeNetRequest`**: the static rulesets of the manifest, the
  dynamic and session rules, compiled into a matcher the service runs
  itself (URL filter with anchors and wildcards, regex filters,
  resource types, domains and initiator domains, request methods, the
  `block`, `allow`, `allowAllRequests`, `redirect`, `upgradeScheme` and
  `modifyHeaders` actions, rule priorities). No extension code runs per
  request: the fast path, and uBO Lite's.
- **`webRequest`**, its events (`onBeforeRequest`,
  `onBeforeSendHeaders`, `onHeadersReceived`, `onCompleted`,
  `onErrorOccurred` and the rest) to the extension's background, and
  its **blocking form** kept, as Firefox keeps it: a listener
  registered with `"blocking"` may cancel or redirect the request, the
  service waiting for the answer, with a deadline (one second, the
  request going on unblocked past it, with a console line) so that a
  hung extension cannot hang the page. The full uBO runs so; the cost,
  a round trip to the extension's renderer for each request of the
  tab, is measured in phase 3 against uBO Lite's rules.

### Cosmetic filtering: a user-origin style sheet

`insertCSS` with `cssOrigin: "user"` (uBO's default for cosmetic
filters) is a style sheet of the CSS cascade's *user* origin, between
the user agent's and the page's, its `!important` above the page's:
`style` gains a user origin beside the user agent and author ones
(today it has two). `cssOrigin: "author"` (the default) is an author
sheet the page cannot see in `document.styleSheets`. `removeCSS`
removes it. Procedural filters (`:has-text`, `:upward` and the like)
are uBO's own code in its content script, needing the DOM and
`MutationObserver`, which exist.

### Storage, permissions, UI

- `storage.local`, `storage.session` and `storage.sync` (sync kept
  local: there is no sync service; Windows only, as the scope says),
  in the browser's storage service (M34), keyed by extension ID, with
  `unlimitedStorage`; `onChanged` to every context of the extension.
- **Permissions**: the manifest's API and host permissions shown at
  install, as Chrome words them, accepted or the install refused;
  `activeTab` for a click on the action; optional permissions by
  `permissions.request`, prompting. Every privileged call is checked
  in the browser against the calling extension's granted permissions,
  never in its renderer.
- **The action**: a toolbar button per extension with an action, its
  icon, badge text and colour (uBO's count) and title; a click opens
  its popup, an extension page in a small window under the button,
  closed when it loses focus. The options page in a tab. Context menus
  (`contextMenus`) in the page's menu, notifications as Windows toasts:
  later phases.
- **Extension pages' CSP**: Manifest V3's: no remote code, no `eval`,
  `wasm-unsafe-eval` allowed (both extensions run WebAssembly).

### Native messaging (optional)

1Password's desktop integration uses `runtime.connectNative` to a
native host registered in the Windows registry
(`HKCU\Software\Google\Chrome\NativeMessagingHosts\<name>`, which
1Password's installer writes for Chrome-family browsers). The browser
process would start the host (its manifest's path, checked to allow
the extension's ID) and pipe length-prefixed JSON. It starts a native
process, so it is a phase of its own, for the lead's ruling after the
rest works; the extension signs in on its own meanwhile.

### Security

- A content script's realm is as trusted as the page's renderer that
  holds it: what it asks of the extension goes as a message, which the
  browser stamps. A compromised page renderer can forge a content
  script's message, but only as the frame it is, which the extension
  sees (as in Chrome).
- The extension's renderer holds the extension's code and data only;
  its API calls are checked by the browser, per permission.
- Requests the extension makes itself are of its origin, with its host
  permissions deciding CORS (as Chrome's extension fetches do).
- An extension cannot read another's storage or pages, or run in
  another's pages (`chrome-extension:` pages match no content script
  of another extension), or in the browser's own pages.

## 2. WebAssembly memory without the copy (G79)

1Password's crypto and uBO's filter structures run in WebAssembly, and
so does Twitch's player: its worker's start, a dozen crossings a second
for some 40 s, takes a core on every Twitch load (2026-11-18), the
two compares of a 32 MB memory at each crossing. The memory's
`ArrayBuffer` should *be* the memory's bytes. This replaces
`Wasm::sync_in` and `sync_out` (the copy in runs that differ, its one
`unsafe` block) and the mirrors.

**The buffer.** `memory.buffer` is an `ArrayBuffer` made by
`JS_NewArrayBuffer` over the `wasmi` memory's bytes (pointer and
length from `Memory::data_mut`), with a free function that frees
nothing (the store owns the bytes) and the buffer's address and the
memory's key kept in the bridge's table of live buffers.

**Every path that can move or free the bytes**, and what happens on it:

1. *Growth inside a call* (`memory.grow` in the module): `wasmi`
   reallocates the bytes. No script runs inside a call (the
   bridge's rule: a call that reaches a script's function stops), so
   the old buffer is detached (`JS_DetachArrayBuffer`) before any
   script runs again: when the call returns, traps, stops at an import
   (the resumable call's host trap), runs out of fuel or budget, or
   panics (the store then dead and every buffer detached). The bridge's
   `leave` already detaches a grown memory's buffer; it becomes the
   only path out of a call, and is checked so (a test per way out).
2. *Growth by a script* (`WebAssembly.Memory.prototype.grow`): the
   native detaches the buffer before growing.
3. *The store dropped* (the realm going, or a panic): every live
   buffer is detached first, then the store dropped; `Drop` of the
   bridge does it, and the realm's own `Drop` (which already empties
   its saved values before its context) calls it.
4. *A script detaching or moving the buffer*: `transfer()`,
   `transferToFixedLength()`, a `structuredClone` or `postMessage`
   transfer list naming it, `ArrayBuffer.prototype.resize`: HTML gives
   a memory's buffer a detach key so these throw `TypeError`; the
   bootstrap's wrappers of them refuse a buffer in the bridge's table,
   and QuickJS-ng's own transfer path is checked to call a hook or the
   buffer is made non-transferable by QuickJS's flag if it has one
   (phase 0 of this section reads QuickJS-ng for every path that frees
   or reallocates an `ArrayBuffer`'s data).
5. *Garbage collection of the buffer*: the free function does nothing;
   the table's entry is removed.

**The `unsafe` argument.** One block makes the buffer
(`JS_NewArrayBuffer` with the store's pointer): sound while the bytes
stay where they are and live, which paths 1 to 5 hold, each by a
detach that precedes any script's next access. One block detaches.
The SAFETY comments name the paths; the shell's allowlist test and
CI's grep count the blocks.

**A fuzz target** (`fuzz/fuzz_targets/wasm_memory.rs`): sequences of
growth (by module and by script), writes and reads through views,
calls with imports that touch the buffer, transfers, `structuredClone`,
and the realm's teardown, run under the Linux job with
AddressSanitizer, its oracle the module's view of the bytes and the
script's agreeing at each step.

**Measures**: the bench's 32 MB call with a byte written goes from
5.7 ms to nearly nothing; Twitch's worker start from some 40 s of a
core to the module's own work.

## 3. Phases, each tested against Chrome

0. **Inventory** (half a day): both extensions' manifests and a scan of
   their packages for every `chrome.*`/`browser.*` call; the list of
   what to build, exact. And QuickJS-ng read for section 2's paths.
1. **Loading** (2 days): CRX3 and unpacked installs, the ID, the
   manifest, the `chrome-extension:` origin and its renderer, the
   background (MV3 worker, MV2 page), `storage`, `runtime` messaging
   within the extension. Tests: a test extension's background answers
   a message; CRX signatures checked (a bad one refused).
2. **Content scripts** (3 to 4 days, needs G90 phases 1 to 3): isolated
   worlds, injection timing, `all_frames`, `match_about_blank`,
   `tabs.sendMessage` by frame, `scripting.executeScript`. Tests
   against Chrome with a test extension: a page's expando unseen by the
   content script; an `about:blank` frame made by script gets the
   script synchronously; messages by `frameId`.
3. **Network** (3 to 4 days): `declarativeNetRequest`, `webRequest`
   with its blocking form and deadline. Tests: rules against Chrome's
   behaviour on the same requests; the round trip measured on the
   sweep's ad-heavy sites (weather, bbc, verge).
4. **Cosmetic CSS and the action** (2 to 3 days): the user cascade
   origin, `insertCSS`/`removeCSS`, the toolbar button, badge and
   popup. Tests: a user-origin rule beating a page's normal one and
   losing to its `!important`, as Chrome.
5. **uBlock Origin runs** (2 days): its package installed, its
   dashboard, its filter lists, blocking on the sweep's sites; the
   sweep counts requests blocked, against uBO in Chrome.
6. **1Password runs** (2 days): its package installed, sign-in in the
   extension (the user signs in), filling a test login form and a
   daily-list sign-in page, by the user in a dogfood session.
7. **WebAssembly zero-copy** (section 2; 3 to 4 days with its fuzz
   target), when the lead orders it among the builds.
8. Later: native messaging (lead's ruling), context menus,
   notifications, the Web Store's updates.

About three to four weeks for phases 0 to 6, after G90's phases 1 to
3, which phase 2 needs.

## 4. Not in this note

- Other extensions' APIs beyond what the two use (refused at install).
- Firefox's XPI format (the full uBO also ships a Chromium MV2 build,
  which the CRX door installs).
- Sync of extension storage across machines (no sync; Windows only).
- DevTools for extensions' pages beyond what the inspector gives any
  page.
