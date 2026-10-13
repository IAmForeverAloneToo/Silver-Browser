# Design note: the security hardening sweep

Roadmap M33. Written before the code, as the record of the decisions,
and corrected to the code when the milestone ends: where they
disagree, the code wins, and section 4 lists the places, marked
*(corrected)*. What
ships is described in `docs/architecture.md` "Current state", and the
threat model this milestone starts lives in `docs/security.md`.

The engine shows one document a tab and nothing inside it: an
`<iframe>` is an inline box whose fallback text is drawn, and
`window.parent`, `top` and `frames` are the window itself. Content
Security Policy is `script-src` alone, with nonces, hashes and
`'strict-dynamic'` matching nothing and `eval` running whatever the
policy says (M24.6). No request carries a `Referer`, no host is
remembered as HTTPS-only, a request from a public page to the local
network is refused without asking (D19), and the sites a document is
embedded under do not exist, so nothing is kept apart by them. The
`unsafe` code of the workspace was never counted, the fuzz corpora live
in a gitignored folder of the machine that grew them, and there is no
threat model. Earlier notes left to this milestone, besides its own
items: the partition of connections, caches, cookies and storage by
top-level site (M14, M27, M28), the third-party cookie policy (M27),
the profile's encryption at rest (M28), revocation, Certificate
Transparency and Encrypted Client Hello (M14), the mixed-form warning
(M27), a worker's own policy and referrer policy (M31), and the frames
that `document.domain` and `postMessage` were waiting for (M27, M31).

This milestone gives the engine frames, kept apart from each other and
from the page that embeds them: an `<iframe>` loads a document of its
own from `src` or `srcdoc`, laid out as a replaced box, drawn into the
page, given the pointer, the keyboard and the wheel when they are over
it or in it, scripted through a window that answers only what HTML
lets another origin ask, with `postMessage` between frames; its
`sandbox` attribute with every flag; `X-Frame-Options` and
`frame-ancestors` refusing a document that does not want to be framed.
Cookies, storage, the HTTP cache and connections are kept apart by the
top-level site a frame is under, with partitioned cookies (CHIPS).
Content Security Policy in full: every fetch directive, nonces,
hashes, `'strict-dynamic'`, `'unsafe-hashes'`, `'unsafe-eval'` and
`'wasm-unsafe-eval'`, `style-src`, `base-uri`, `form-action`,
`sandbox`, `upgrade-insecure-requests`, report-only policies,
`securitypolicyviolation` events and reports to `report-uri`.
Referrer Policy, from every place a policy is given, and `Referer` and
`document.referrer` by it. HSTS remembered in the profile, with the
preload list shipped read-only; the `Upgrade-Insecure-Requests` field;
a warning before a form leaves a secure page in the clear. Local
Network Access's permission prompt. The profile's cookies and storage
encrypted at rest. The workspace's `unsafe` in one place with its
argument, every other crate forbidding it, and the third-party
`unsafe` inventoried. The fuzz corpora minimized and committed, and
new targets for what this milestone parses. A threat model,
`docs/security.md`, and the adversarial test site's second version,
where every attack this milestone defends against must fail closed.

The standards are read at the versions current when this note was
written (13 October 2026):

- the HTML Standard (Living Standard): 4.8.5 (the `iframe` element,
  `srcdoc`, `sandbox`, `allow`), 7.1 (sandboxing and its flags),
  7.2.2 and 7.3 (navigables, the window proxy, cross-origin objects:
  `CrossOriginProperties`, `IsPlatformObjectSameOrigin` and the
  `SecurityError`s), 7.4 (navigation: choosing a navigable, allowed by
  sandboxing to navigate, the `X-Frame-Options` processing, the
  ancestor checks), 9.4 (cross-document messaging), 15.4.4 (the
  rendering of `iframe`);
- Content Security Policy Level 3 (W3C Working Draft, and the Editor's
  Draft where it moved on);
- Referrer Policy (W3C Candidate Recommendation, and the Editor's
  Draft), and Fetch's "determine request's referrer";
- RFC 6797 (HTTP Strict Transport Security), and the Chromium preload
  list's format;
- Upgrade Insecure Requests (W3C Candidate Recommendation); Mixed
  Content Level 2;
- Local Network Access (WICG draft of 7 August 2026);
- Cookies Having Independent Partitioned State (CHIPS, the Privacy CG's
  draft) with RFC 6265bis, and the Storage Standard's storage keys;
- the Fetch Standard (Living Standard) for the requests of frames,
  partition keys and the order of the checks.

What the standards leave to the implementation, or state but are worth
checking, was put to the reference browser (Chrome 154.0.8037.99,
headless) before this note was written, from a server of two sites on
two ports (`a.localhost` and `b.localhost`, scratchpad `m33/`, pages
`p1-frames` to `p8-lna`):

- *frames* (p1): an `<iframe>` with no size is 304 by 154 (300 by 150
  and a border of `2px inset rgb(0, 0, 0)`), `display: inline`,
  `vertical-align: baseline`, `overflow: clip`; `width` and `height`
  attributes size its content box. Its first document is
  `about:blank`, complete, `<body></body>`, of the embedding origin,
  there as soon as the element is; a `srcdoc` wins over `src`, its
  document's URL `about:srcdoc`, its base URL the parent's, its origin
  the parent's; a frame of another origin is `about:blank` until its
  document arrives. `window.length` counts the frames, `frames[i]` and
  `frames.name` (by the `name` attribute) are their windows, `parent`
  and `top` the embedder's, `frameElement` the element. A frame of
  another origin: `contentDocument` null; `document`, `name`,
  `location.href`, `frameElement` and `toString` throw `SecurityError`
  ("Failed to read a named property 'document' from 'Window': Blocked
  a frame with origin "http://a.localhost:8331" from accessing a
  cross-origin frame."); `length`, `closed`, `parent`, `top` answer;
  its own property names are exactly `window`, `self`, `location`,
  `closed`, `frames`, `length`, `top`, `opener`, `parent`, `blur`,
  `close`, `focus`, `postMessage` and `then`, its location's `href`,
  `replace` and `then`. The parent's `load` waits for its frames';
  a message's `source` is the sender's window, its `origin` the
  sender's. The frames' requests carry `Sec-Fetch-Dest: iframe`, the
  parent as referrer by the default policy (`http://a.localhost:8331/`
  across sites, the whole URL within the origin), and
  `Upgrade-Insecure-Requests: 1`, as every navigation does;
- *sandbox* (p2): `sandbox.supports()` knows thirteen tokens:
  `allow-downloads`, `allow-forms`, `allow-modals`,
  `allow-orientation-lock`, `allow-pointer-lock`, `allow-popups`,
  `allow-popups-to-escape-sandbox`, `allow-presentation`,
  `allow-same-origin`, `allow-scripts`, `allow-top-navigation`,
  `allow-top-navigation-by-user-activation` and
  `allow-storage-access-by-user-activation`; tokens match in any case
  and unknown ones are ignored. `sandbox` alone runs no script and
  makes the frame's origin opaque (`contentDocument` null even of the
  same origin); `allow-scripts` alone runs scripts with `origin`
  "null", whose `document.cookie`, `localStorage` and `sessionStorage`
  throw `SecurityError` ("The document is sandboxed and lacks the
  'allow-same-origin' flag."), `document.domain` empty, and whose
  messages come from origin "null"; with `allow-same-origin` the
  origin is kept. A form is not submitted without `allow-forms`. A
  frame inside a sandboxed frame is sandboxed too, its request sent
  with no referrer and `Sec-Fetch-Site: cross-site`. A
  `srcdoc` sandboxed is `about:srcdoc` of origin "null". The
  `Content-Security-Policy: sandbox allow-scripts` header does what
  the attribute does. A change of the attribute applies at the
  frame's next navigation;
- *framing refused* (p3): `X-Frame-Options: DENY` refuses the frame;
  `SAMEORIGIN` allows it when every ancestor is of the frame's origin
  (a frame of `a` inside a frame of `b` inside `a` is refused);
  `ALLOWALL`, `ALLOW-FROM`, an unknown value and none allow; several
  values (`DENY, SAMEORIGIN`, two `DENY` fields, `DENY, bogus`) refuse
  unless they are one value repeated (`SAMEORIGIN, sameorigin`
  allows). `frame-ancestors 'none'` refuses, `'self'` refuses another
  origin's parent, a matching source allows; a `frame-ancestors`
  directive overrides `X-Frame-Options`; in a report-only policy, or a
  `<meta>`, it refuses nothing, and neither does a `<meta>`
  `X-Frame-Options`. A refused frame fires `load` on its element and
  shows an error page of another origin (`contentDocument` null,
  `location` throwing);
- *CSP* (p4, 25 policies): every violation is reported by a
  `SecurityPolicyViolationEvent` that bubbles and is composed, at the
  element when one is to blame (`SCRIPT`, `STYLE`, `BUTTON`, `DIV`)
  and at the document otherwise, with `violatedDirective` equal to
  `effectiveDirective` (`script-src-elem`, `script-src-attr`,
  `style-src-elem`, `style-src-attr`, `img-src`, `connect-src`,
  `frame-src`, `worker-src`, `form-action`, `base-uri`, and
  `script-src` for `eval` and WebAssembly), `blockedURI` the URL or
  `inline`, `eval` or `wasm-eval`, `disposition` `enforce` or `report`,
  `sourceFile`, `lineNumber` and `columnNumber` where the violation
  happened (none for a parser-inserted external script's URL), a
  `sample` of the first 40 characters only with `'report-sample'`,
  `statusCode` 200. Nonces: a matching one runs, `script.nonce` keeps
  it while `getAttribute('nonce')` gives "" (nonce hiding, the
  attribute still there), a nonce with a leading space matches.
  Hashes: `sha256` and `sha384` of an inline script's exact text, a
  space more fails; an external script is not allowed by the hash of
  its body. `'strict-dynamic'`: a script a nonced script makes and
  inserts runs; a parser-inserted one, one `document.write` writes,
  and inline scripts with `'unsafe-inline'` do not. `eval`, indirect
  `eval`, `Function` and the async and generator function
  constructors throw `EvalError` ("Evaluating a string as JavaScript
  violates the following Content Security Policy directive because
  'unsafe-eval' is not an allowed source of script: ..."), `eval`
  and `Function` remaining themselves (`eval === window.eval`);
  `setTimeout` and `setInterval` with a string return an id and run
  nothing; in report-only all of them run and report. WebAssembly's
  `Module`, `compile` and `instantiate` throw `CompileError` without
  `'wasm-unsafe-eval'` or `'unsafe-eval'`, `validate` still answers.
  `style-src 'none'` stops a `<style>` and a `<link>` (each fires
  `error`), a `style` attribute in the markup and one set by
  `setAttribute`, while `element.style.color = ...` and `cssText` set
  through the CSSOM apply; a nonced `<style>` and a hashed one apply.
  `connect-src 'none'` fails `fetch` with "TypeError: Failed to
  fetch", an XHR with `error` and status 0, a synchronous one by
  throwing `NetworkError`. `frame-src` refuses a frame as framing does
  (`load` fires, an error page), `child-src` stands for `frame-src` and
  `worker-src`, a worker refused fires `error` on the `Worker`. A
  worker's own response's policy governs its fetches. `form-action
  'none'` stops a submission, `base-uri 'self'` a `<base>` of another
  origin. Two policies in one header must both allow. A `<meta>` policy
  in the body does nothing. `report-uri` sends a POST of
  `application/csp-report` with `document-uri`, `referrer`,
  `violated-directive`, `effective-directive`, `original-policy`,
  `disposition`, `blocked-uri`, `line-number`, `source-file`,
  `status-code` and `script-sample`. `upgrade-insecure-requests` does
  not upgrade a request to a `localhost` name;
- *referrer* (p5): the default is `strict-origin-when-cross-origin`;
  each of the eight policies as Referrer Policy defines them, from a
  `Referrer-Policy` header (the last value understood wins:
  `no-referrer, origin` is `origin`, `origin, bogus` is `origin`), a
  `<meta name=referrer>` (which applies from where it is parsed on,
  `never` meaning `no-referrer`, an unknown value ignored), a
  `referrerpolicy` attribute on `img`, `script`, `link`, `iframe` and
  `a` (an unknown value ignored), `rel=noreferrer`, and `fetch`'s
  `referrerPolicy`; `fetch`'s `referrer` takes a URL of the same origin
  and falls back to the document's otherwise; `new Request(u).referrer`
  is "about:client", its `referrerPolicy` ""; a bad policy is a
  `TypeError`; a redirect's `Referrer-Policy` applies to the next hop;
  `document.referrer` of a page the user opened is "";
- *frames navigating* (p6): a frame may navigate a frame of its own
  origin anywhere in the tab, its descendants, and the top; another
  origin's frame elsewhere gives `SecurityError` ("The current window
  does not have permission to navigate the target frame to '...'").
  A cross-origin frame navigates the top without a user's activation;
  a sandboxed one without `allow-top-navigation` throws, with it
  navigates. `postMessage` to a target origin that does not match is
  dropped silently, `"/"` meaning the sender's own origin, a target
  that is not a URL a `SyntaxError`. A frame's `window.name` set by
  its script is not a name its parent finds it by when the frame is of
  another origin;
- *partitions* (p7): a frame of `b` inside `a` sees none of the
  `localStorage` and `sessionStorage` `b` wrote as a top-level page;
  it is sent and shows the cookies `b` set `SameSite=None; Secure`
  (third-party cookies are on), not the `Lax` ones nor the default;
  a `Partitioned` cookie set by `b` at the top is not there, and one
  set in the frame is the frame's (and sent with its later requests)
  only under `a`;
- *Local Network Access* (p8, with the page's address forced public):
  `fetch`es to a loopback address fail with "Failed to fetch" at once
  in the headless browser, which denies the permission; the
  permission names `local-network-access` and `loopback-network`
  answer `denied` there and `local-network` `prompt`.

## 1. Decisions

### Frames

**Each frame is an engine of its own, under its parent's (D24,
M33.2).** Three ways were weighed:

- *The engine split into a tab and a tree of navigables*: the
  document's state that `Engine` keeps beside `Shown` (the
  navigation, the viewport, the scroll goal) moved into a navigable,
  every method of the engine taking the navigable it acts on, and the
  results of fetches, decodes and events routed by page. The engine's
  methods name `self.shown` some 470 times; the tests that pin the
  incremental counters (M13, M22) all pass through them.
- *One runtime for each set of same-origin frames now*, so that a
  parent's script can reach into a frame's document as HTML allows
  for one origin: QuickJS-ng can hold several contexts in a runtime,
  but every realm here is a runtime of its own (M24), lends one
  document for a run, and names its nodes by 53 bits of `NodeId` with
  none left for a document. It is the design M34's process for a site
  instance will need, and doing it here would mean doing it twice.
- *A child engine for each frame*: the `Engine` type already is a
  document in a viewport, with its loading, styles, layout, painting,
  realm, input, scrolling and lifecycle; an `<iframe>` gets one,
  owned by the document that has the element, sharing the tab's
  fonts, cookies, cache, storage, clock and waker, and the parent
  draws its pixels as it draws a canvas's, gives it the input that
  falls on it, and passes messages between them through a frame tree
  the tab shares.

The third is chosen. It is what a site-isolating browser's frame is
seen as from its parent anyway (another document drawn into a
rectangle, reached by messages), so M34 can move a child engine into a
renderer process of its own behind the same seam. Its price is the one
the second way would pay: **every frame is cross-origin to every other
frame**, the same origin included, until M34's site instances share a
runtime (registered as a gap, G90). A same-origin frame's
`contentDocument` is null and its window answers what another
origin's does; a script that reaches into its own origin's frame gets
the `SecurityError` HTML gives across origins. That is the closed
side of HTML's rule, never the open one: no frame reads anything of
another that a cross-origin frame could not.

**The frame tree.** A tab's frames are kept in a tree the engines
share (`Arc<Mutex<FrameTree>>`): for each frame its number, its parent,
its children in the order of their elements in the parent's document,
its name (the `name` attribute, or a name its script set while it was
of the parent's origin), its document's origin and URL, its sandboxing
flags, and an inbox of what other frames sent it (messages and
navigations). The top-level document is frame 0. A frame's number is
its navigable's: it stays when the frame navigates, and the window
objects scripts hold for it are made from it, so `frames[0] ===
f.contentWindow` holds across navigations. What a script asks of
another frame at once (its `length`, its `parent`, its `top`, a child
by index or name, `closed`) is read from the tree under its lock; what
it asks to happen (a message, a navigation) goes into the target's
inbox, and the engine owning the target takes it at its next task
turn, waking the shell as a fetch's result does.

**The `iframe` element.** When an `<iframe>` is connected to a
document of an engine (with or without scripts), the engine makes its
child engine, showing the initial `about:blank` of the parent's origin,
and then processes the attributes as HTML 4.8.5 does: `srcdoc`, if
present, is loaded as a document of URL `about:srcdoc`, whose base URL
is the parent's and whose origin and policy container (CSP list and
referrer policy) are the parent's (an opaque origin if sandboxed);
else `src`, parsed against the parent's base URL, is navigated to,
unless it is empty or `about:blank`, which keeps the initial document.
A change of `src` or `srcdoc` navigates again; `name` renames;
`sandbox` takes effect at the next navigation (as the reference
browser does). Removing the element unloads and drops the child
engine (its `pagehide` and `unload` run first); moving it does the
same and makes a new one, as HTML's removal and insertion steps do.
The navigation request is the child's, made as the parent's document
initiated it: destination `iframe` (`Sec-Fetch-Dest: iframe`, which
`net` learns), mode `navigate`, the parent as initiator for mixed
content and Local Network Access, the parent's referrer by its policy
or the element's `referrerpolicy`, and the top-level site for cookies
and partitions (below). A URL whose document an ancestor already shows
(fragments aside) is not loaded, HTML's guard against a frame embedding
itself.

**Limits.** At most 100 frames in a tab (`MAX_FRAMES`) and 16 levels of
nesting (`MAX_FRAME_DEPTH`); an iframe past either stays empty, and the
console says why. The reference browser allows 1,000 frames a page; a
frame here is a whole engine with its own realm and threads, and 100 is
past what the pages measured for this note use (their advertising
frames are tens). A frame's realm has a heap of 64 MiB (a top-level
document's is 256), its fetches six threads at most (the top's 32),
and its decoder two. A frame's inbox keeps 1,024 entries; what comes
past them is dropped with a console line. These go in
`docs/security.md`'s table of limits.

**Layout.** `Replaced::Frame { node }` in `layout`, made by
`replaced_of` for an `iframe` element: a replaced box with no natural
size, so 300 by 150, whose children are not laid out (the fallback
text the tokenizer makes of an iframe's content is never shown). The
user agent sheet gets HTML 15.4.4's `iframe { border: 2px inset; }`
(`overflow: clip`, as the reference browser computes), and `width`,
`height` and `frameborder` are presentational hints (a `frameborder`
of 0, `no` or an integer that is 0, removes the border). The child's
viewport is the frame's content box: its media queries, its
`innerWidth`, its `resize` event follow the box, and a change of the
box lays the child out again.

**Drawing.** The parent's display list has an `Image` item for the
frame's content box (`frame_image(node)`, bit 60 of the id, free since
M32), clipped to it; when the parent shows its pixels it asks the
child for a frame of the box's size in device pixels at the tab's
scale, and the child draws one if it changed since the last (its own
layout, scroll, animations and scrollbar), in software. The image's
frame number is the child's count of frames drawn, so both backends
keep its texture until the child draws again. A frame's drawing is
snapped to device pixels as M32's svg is. The child always draws in
software, also when the tab is on the GPU, whose backend uploads the
pixels as it does a canvas's: composing a frame's own layers on the GPU
is M34's, when its pixels come from another process (G91). Printing
draws a frame as it is on the screen, at the print's scale.

**Input.** The parent's hit test that lands on a frame's content box
hands the event to the child at the point inside it: pointer moves,
presses and releases (a press that began in a frame keeps the frame
until it is released, as capture would), the cursor and the link under
the pointer, the wheel (the child scrolls first; what it cannot take
goes to the parent's scroll chain), clicks and focus. A click in a
frame focuses it: the parent's focused element becomes the iframe
(`document.activeElement` is the iframe in the parent, the child's
document has focus), and keys go to the child's focused element.
Sequential focus enters a frame where its element is in the order and
leaves it after the child's last focusable element, through the
`KeyOutcome::FocusLeft` the engine already gives the browser's
interface. The frame's border and the parent's scrollbar stay the
parent's.

**Lifecycle and tasks.** The parent's `load` waits for its frames'
(HTML's "delaying the load event"), `DOMContentLoaded` does not. A
frame's `load` event fires on its element when the child's document
finishes loading, also for an error page and for a refused frame; for
an initial `about:blank` it is queued as a task when the element is
inserted (the reference browser fires it within the insertion, while
the script that inserted it runs: a recorded deviation). The parent's
task turns, its timers, its rendering steps (`requestAnimationFrame`,
resize, scroll events) and its network polling run its children's in
tree order; the next time a task is due, and whether a frame is
wanted, are the earliest and any of the tree's. The console of a frame
is the tab's console, each line saying which frame's URL it came
from. A frame's navigations are its own: they are not entries of the
tab's history, and back and forward move the top-level document only
(the joint session history, G92); its back-forward cache holds
nothing, and a frame is kept in its parent's cache with the parent.

**Scripts across frames.** HTML's `WindowProxy` for another frame, as
cross-origin objects are (7.2.3): an object of the bootstrap's whose
properties are `window`, `self`, `location`, `closed`, `frames`,
`length`, `top`, `opener` (null), `parent`, `blur`, `close` (a no-op
for a frame), `focus`, `postMessage`, `then` (undefined) and indices,
everything else throwing the reference browser's `SecurityError`; and
a cross-origin `Location` of `href` (to set) and `replace`. Every frame
is cross-origin here (above), so these are what `contentWindow`,
`parent`, `top`, `frames[i]` and named frames give; one object a
frame a realm, so identity holds. `window.length` and `frames[i]` count
and give the document's child frames, `window.name` and named access
find a child by its element's `name` (and by a name its own script set
while it was same-origin, which here is never), `frameElement` and
`contentDocument` are null, `opener` null. `postMessage(message,
targetOrigin, transfer)` to another frame: the message cloned as M31
clones (a transfer list of ports included), the target origin parsed
(`"*"`, `"/"` for the sender's own, a URL's origin, `SyntaxError`
otherwise), the message put in the target's inbox, and delivered as a
`MessageEvent` only if the target's document's origin then matches,
its `origin` the sender's serialization, its `source` the sender's
window in the target's realm. Setting another frame's `location` (or
`href`, or calling `replace`) navigates it when the caller may
navigate it: the target is the caller or one of its descendants, or
the top (unless sandboxed), or a frame some ancestor of which (itself
included) is of the caller's origin, which is what the reference
browser allows; else the `SecurityError` above. Link and form targets
choose a navigable by HTML 7.3.1: `_self`, `_parent`, `_top`, and a
name among the frames the source may navigate; `_blank` and a name
found nowhere, which ask for a new tab the browser does not have yet
(M37), navigate the top for a frame (the nearest to what a user
clicking it expects) and keep today's behaviour at the top; a
sandboxed frame without `allow-popups` gets nothing for them, with a
console line.

**`document.domain`.** Its getter as now; its setter changes nothing
across frames, which are cross-origin whatever it says (the reference
browser's origin-keyed agent clusters, its default since 2023, have
the same effect); in a sandboxed document without
`allow-same-origin` it throws `SecurityError`, as HTML's "sandboxed
document.domain browsing context flag" says.

### Sandboxing and refusing to be framed

**Sandboxing flags (M33.2).** The `sandbox` attribute's tokens are
parsed in any case, the unknown ignored, into HTML 7.1's flag set, the
frame's flags being its attribute's at navigation, its parent's own
active flags, and its document's CSP `sandbox` directive (a header's,
never a `<meta>`'s or a report-only one's; it applies to top-level
documents too). `HTMLIFrameElement.sandbox` is a `DOMTokenList` whose
`supports()` knows the reference browser's thirteen tokens. What each
flag guards here:

- *scripts* (no `allow-scripts`): the child engine's scripting is off
  for that document: no script, no event handler attribute, no
  `javascript:` URL; `<noscript>` is parsed as when scripts are off;
- *origin* (no `allow-same-origin`): the document's origin is a new
  opaque one; `document.cookie`, `localStorage`, `sessionStorage` throw
  the `SecurityError`s above, `document.domain` is "", the requests it
  makes have origin `null`, no referrer to send (an opaque origin's
  URL is not sent; the requests seen in p2 had none) and are
  cross-site to everything;
- *forms* (no `allow-forms`): a submission is not made, with the
  reference browser's console line;
- *navigation* (always, but for the frame itself and its
  descendants) and *top-level navigation* (no `allow-top-navigation`,
  or no `-by-user-activation` with an activation): navigating the top,
  or a frame the frame does not contain, throws the `SecurityError`
  above;
- *auxiliary navigation* (no `allow-popups`): `_blank` and unknown
  targets give nothing; `window.open` is G (there is none);
- *modals* (no `allow-modals`): `alert`, `confirm`, `prompt` and
  `print` do nothing and say so on the console;
- *downloads* (no `allow-downloads`): a navigation whose response would
  be a download is dropped (the engine saves none yet, M37.2, so this
  changes the notice only);
- *`document.domain`* (above);
- *pointer lock, orientation lock, presentation, storage access by
  user activation, automatic features, plugins, custom protocols*:
  nothing in the engine they would guard; their tokens are known.

`allow-popups-to-escape-sandbox` has nothing to let escape until the
browser has popups.

**`X-Frame-Options` and `frame-ancestors` (M33.2).** When a child's
response arrives, before anything of it is parsed: if a policy of the
response's `Content-Security-Policy` fields (not a `<meta>`'s, and not
a report-only one's, which reports) has a `frame-ancestors` directive,
each ancestor's origin, from the parent to the top, must match its
source list (`'none'`, `'self'`, scheme and host sources; never
`'unsafe-inline'`, nonces or hashes); otherwise `X-Frame-Options` by
HTML 7.4.2.3.7's processing model, as the probes found: the values of
all its fields split on commas, trimmed and lowercased; none, or
`allowall`, or a value it does not know (including `allow-from`),
allows; several distinct values refuse; `deny` refuses; `sameorigin`
allows only when every ancestor is of the frame's origin. A refused
frame shows the engine's error page ("This page refused to be shown in
a frame"), of an opaque origin, fires its element's `load`, and says
on the console why. A top-level document is never refused by them.

### Partitions by top-level site

**Third-party cookies on, partitioned state for the rest (D25).** The
choice is the reference browser's as the probes found it: a frame of
another site sends and reads the cookies its site set
`SameSite=None; Secure`, and keeps everything else apart by the
top-level site it is under. Blocking third-party cookies (as Safari
does) and partitioning all of them (Firefox's Total Cookie Protection)
were weighed: both are more private and both break the sign-ins that
embed another site's frame, which the reference browser keeps working
and which M27's acceptance (a page reading another origin by CORS with
its cookies) holds. Blocking them becomes a setting with the browser's
settings (M37.3), registered so (G93).

- *Site for cookies*: a request is same-site when its URL's site is the
  top-level site and every ancestor's document is of that site (the
  RFC 6265bis "site for cookies" with HTML's ancestors); a frame of
  `b` under `a` is cross-site, so `SameSite=Lax` and `Strict` cookies
  are neither sent with its requests nor shown to its `document.cookie`,
  and its navigation request is cross-site too (today the jar takes
  every navigation as top-level).
- *Partitioned cookies (CHIPS)*: a cookie with `Partitioned` (and
  `Secure`, else it is refused) is kept under the top-level site of
  the context that set it (for a top-level document, its own site),
  and sent and shown only in contexts of that partition; the jar's
  limits count each partition apart; they are kept in the profile with
  their partition.
- *Storage*: `localStorage` and `sessionStorage` are keyed by the
  Storage Standard's storage key, the origin and, for a context of
  another site than the top, the top-level site; a first-party frame
  shares its origin's area. The `storage` event goes to documents of
  the same key.
- *The HTTP cache* is keyed by the top-level site and the frame's site
  (the "triple key" the reference browser has since 2021), not by the
  requesting document's site alone.
- *Connections and TLS sessions*: the pool keys a connection by the
  top-level site and whether the request is cross-site to it (Fetch's
  network partition key); rustls's session cache is kept per
  partition, so a resumed session cannot tell one top-level site what
  another visited.

### Content Security Policy (M33.1)

**The policy model.** `net::csp` is rewritten from M24.6's script-only
form: a `Policy` has its directives with their values parsed once, its
disposition (`enforce` or `report`), its source (header or `<meta>`),
and its serialization (for reports); a source list is a list of
`'none'`, `'self'`, `'unsafe-inline'`, `'unsafe-eval'`,
`'wasm-unsafe-eval'`, `'unsafe-hashes'`, `'strict-dynamic'`,
`'report-sample'`, nonces, hashes (`sha256`, `sha384`, `sha512`, by
`aws-lc-rs`'s digests, already in the tree under `rustls`), scheme
sources, host sources and `*`. Directives the engine does not know are
kept and consulted by nothing, as CSP says; `require-trusted-types-for`
and `trusted-types` say on the console that Trusted Types are not
there (G94); `block-all-mixed-content` is a no-op (mixed content is
blocked anyway); `plugin-types`, `navigate-to` and `prefetch-src`
are gone from the draft and ignored. A violation records what a report
and an event need: the directive violated and the effective one, the
blocked URI or `inline`/`eval`/`wasm-eval`, the disposition, the
policy's serialization, the sample, the source position, and the
element to blame. Every policy of the list is checked, not the first
failing alone: one report a policy, enforce ones deciding.
 **Delivery.** A document's policies are its response's
`Content-Security-Policy` fields (enforce) and
`Content-Security-Policy-Report-Only` fields (report), each comma-split
into policies; a `<meta http-equiv=Content-Security-Policy>` in the head
(never a report-only one, never in the body), taken in the first time a
check looks after the parser inserted it, without `frame-ancestors`,
`sandbox` and `report-uri`. Every kind of document gets its response's,
SVG, XML and text documents included (the documents taken whole had
none). A document whose URL is local (`about:blank`, `about:srcdoc`, the
result of a `javascript:` URL, a `data:` URL's document) inherits the
policy container of its creator (the parent for a frame, the document
for a `javascript:` URL), as HTML's policy containers do: so a `srcdoc`
frame of a page with a strict policy runs none of its inline scripts
(p4). A dedicated worker's policies are its script response's, or its
creator's for a local URL (M31 left them to here). `MAX_POLICIES` stays
64 a document.

**Fetch directives (CSP 6.1), at every hop of every request.** The
effective directive by destination, and its fallback list, as CSP3
writes them: `script`, `worker` imports and XSLT `script-src-elem`
(then `script-src`, `default-src`); `style` `style-src-elem` (then
`style-src`, `default-src`); `image` `img-src`; `font` `font-src`;
`iframe` and `frame` `frame-src` (then `child-src`, `default-src`);
`worker` `worker-src` (then `child-src`, `script-src`, `default-src`);
an empty destination (`fetch`, XHR, a beacon, an `EventSource`)
`connect-src`; `manifest` `manifest-src`; `audio`, `video`, `track`
`media-src`; `object` and `embed` `object-src`. The check is `net`'s,
before each hop, from the policies and the request's nonce and parser
metadata (new fields of `net::Request`): a URL matches a source list
as CSP 6.7.2 says (M24.6's matching kept), a nonce source matches the
request's nonce for `script-src-elem` and `style-src-elem`, and
`'strict-dynamic'` makes a script's list ignore host and scheme
sources and `'self'` and `'unsafe-inline'`, allowing a script that is
not parser-inserted. Report-only policies report and never block. What
fails is a network error, which each kind already turns into its
failure: an image broken with `error`, a stylesheet failed with
`error`, a script not run with `error`, a `fetch` rejected with
`TypeError` and an XHR's `error`, a frame showing the refused page, a
worker's `error`. The post-response check stays unmade: with no
service worker the response's URL is the last hop's, which the
pre-request check saw.

**Inline content and code (CSP 6.7.3, 4.2).**

- *Scripts*: an inline `<script>` runs when a policy allows it by
  nonce (the element's `[[CryptographicNonce]]` equal to a nonce
  source, after the reference browser's trimming of white space; a
  nonce in an element whose attributes look like a dangling-markup
  injection, `<script` or `<style` in an attribute name or value, is
  not trusted, as CSP 6.7.3.1 says), by hash of its text (the exact
  text, UTF-8), or by `'unsafe-inline'` where no nonce, hash or
  `'strict-dynamic'` disables it. An external script's request carries
  its nonce and whether the parser inserted it. A hash allows no
  external script (that needs Subresource Integrity, G43).
- *Nonce hiding*: an element's nonce is kept in an internal slot when
  the element is made or its `nonce` attribute set, and when the
  element is connected to a document whose policies came from a
  header the attribute's value is emptied (the attribute stays), as
  HTML's nonce attribute steps say; `element.nonce` reads the slot.
  So a selector like `script[nonce^=a]` cannot read a nonce out.
- *Styles*: a `<style>` element's sheet is made only when
  `style-src-elem` (then `style-src`, `default-src`) allows it by
  nonce, hash of its text or `'unsafe-inline'`, else it fires `error`;
  a `<link rel=stylesheet>`'s request carries its nonce. A `style`
  attribute given by the markup or `setAttribute` is taken only when
  `style-src-attr` (then `style-src`, `default-src`) allows it by
  `'unsafe-inline'`, or by its hash with `'unsafe-hashes'`; one set
  through the CSSOM (`element.style.x`, `cssText`, `setProperty`) is
  not checked, as the reference browser does.
- *Event handler attributes*: compiled only when `script-src-attr`
  (then `script-src`, `default-src`) allows by `'unsafe-inline'`, or
  by the hash of the attribute's value with `'unsafe-hashes'`.
- *`javascript:` URLs*: run only when `script-src-elem` allows
  `'unsafe-inline'`, or the hash of the whole URL with
  `'unsafe-hashes'`.
- *Code from strings* (`'unsafe-eval'`): QuickJS-ng has no hook for
  it (M24.6), and the `js` crate writes no `unsafe` to make one, so
  the bootstrap does it: when a document's policies forbid `eval` (or
  a report-only one would), the realm's `eval`, `Function`, and the
  constructors of async, generator and async generator functions
  (reached through their prototypes' `constructor`) are replaced by
  functions of the bootstrap's that ask the host, which checks the
  policies, records the violation (with the sample and the caller's
  line and column from the stack) and answers; refused, they throw the
  reference browser's `EvalError`; allowed by a report-only policy,
  they evaluate through the originals, kept in the bootstrap's
  closure alone. The replacements keep the originals' names, lengths
  and prototypes (`eval === window.eval`, `Function.prototype.
  constructor === Function`, `instanceof Function`), and nothing a
  page can reach holds an original. A report-only policy's
  replacement evaluates indirectly, so a direct `eval` loses its
  caller's scope while such a policy is there: a recorded deviation.
  A page with no such policy keeps the originals untouched.
  `setTimeout` and `setInterval` with a string ask the same, and when
  refused return an id and run nothing.
- *WebAssembly* (`'wasm-unsafe-eval'` or `'unsafe-eval'`): the `js`
  crate's `WebAssembly.Module`, `compile`, `instantiate` from bytes
  and the streaming forms ask the host before compiling and throw the
  reference browser's `CompileError` when refused; `validate` is not
  checked.
- *`base-uri`*: a `<base href>` whose URL its list does not allow is
  not used (the document's fallback base URL stays).
- *`form-action`*: a form's submission, and each of its redirects, is
  checked against it; refused, nothing is sent.
- *`upgrade-insecure-requests`*: a document with it upgrades its
  subresource requests and its same-origin navigations from `http:`
  to `https:` (port 80 to 443) before mixed content is looked at,
  except to `localhost` names and loopback addresses, as the reference
  browser does; its frames inherit it.

**Violations.** Each violation is reported three ways, each policy's
apart: a `securitypolicyviolation` event
(`SecurityPolicyViolationEvent`, with its constructor and dictionary)
queued as a task at the element to blame when it is connected, else at
the document (at the worker's global in a worker), bubbling and
composed, with the fields the probe listed; a console line, kept in
the engine's words ("Content-Security-Policy refused ..."), saying
`report-only` for those; and, when the policy has `report-uri`, a POST
of `application/csp-report` with the reference browser's JSON to each
URL (resolved against the document's URL, sent with no credentials
beyond the same origin's, by `net`, after the event, at most 100 a
document, its response ignored). `report-to` and the Reporting API
(`Reporting-Endpoints`) are G95. `'report-sample'` adds a sample of 40
characters to the event and the report, of an inline script, style,
attribute, `javascript:` URL or code string.

### Referrer Policy (M33.3)
 **The policy and the referrer.** `net::referrer`: the eight policies
and the empty string, parsed from a `Referrer-Policy` field's
comma-split tokens (the last one known wins), a `<meta name=referrer>`
(its legacy names `never`, `default`, `always` and
`origin-when-crossorigin` mapped as HTML does; it applies from where it
is parsed), and `referrerpolicy` attributes (unknown values ignored);
"determine request's referrer" (Referrer Policy 8.3) with its stripping
(no credentials, no fragment; the origin alone where the policy says or
the URL is longer than 4,096 bytes; nothing for an opaque origin or a
local scheme), the downgrade from a secure URL to one that is not
potentially trustworthy, and the default
`strict-origin-when-cross-origin`. `net::Request` gains `referrer`
(none, the client's, or a URL) and `referrer_policy`; `net` sets the
`Referer` field when the request starts and again at each redirect,
whose `Referrer-Policy` replaces the request's policy for the next hop
(Fetch 4.4). `referer` stays a forbidden name for scripts.

**Who gives which.** A document's policy is its response's
`Referrer-Policy`, then each `<meta name=referrer>` as it is parsed;
`about:blank`, `about:srcdoc` and other local documents inherit the
creator's, and a worker takes its script response's. Its requests use
it with the document's URL as referrer: subresources (an element's
`referrerpolicy` overriding: `img`, `script`, `link`, `iframe`, and
`a` and `area` for their navigations; `rel=noreferrer` on `a`,
`area` and `link` meaning `no-referrer`), a stylesheet's own requests
(its imports and images: the sheet's URL as referrer, the document's
policy), navigations the document starts (links, forms, `location`,
a frame's `src`), `fetch()` and XHR (with `Request`'s `referrer` and
`referrerPolicy`: a URL of another origin falls back to the client's,
`about:client` is the default, the enum checked as the IDL does),
reports and images of the cache's revalidation. A navigation the user
starts (typed, a bookmark, back and forward to a page) has none.
`document.referrer` is the referrer of the request that brought the
document (the `Referer` sent, as a string), or for a local document its
creator's URL, or "".

### HSTS and Upgrade-Insecure-Requests (M33.3)

**Strict Transport Security.** `net::hsts`: the
`Strict-Transport-Security` field parsed by RFC 6797 6.1 (`max-age`
required, `includeSubDomains`, quoted values, names in any case,
unknown directives ignored, a directive twice or a bad `max-age`
making the field void), taken only from an `https:` response over a
connection whose certificate verified (every connection here, which
fails closed otherwise) and whose host is a domain name, not an IP
address; `max-age=0` forgets the host. A known host is one that
matches a stored entry exactly, or as a subdomain of one with
`includeSubDomains`, or the preload list likewise. A request to
`http:` of a known host, at any hop, a navigation's included, becomes
`https:` before anything is sent (port 80 to 443, other ports kept),
as Fetch's main fetch upgrades it; a certificate error on such a host
was never bypassable here, so nothing more is needed for RFC 6797
12.1. The store is the tab's (shared with frames and workers like the
cookies), kept in the profile in a table of its own (host, expiry,
`includeSubDomains`), the expiry an absolute time by the engine's
clock; at most 10,000 hosts, the soonest to expire dropped first.

**The preload list, shipped read-only (D26).** The reference browser's
list (`transport_security_state_static.json`, Chromium's, under its
BSD licence) is the web's: sites ask to be on it through
hstspreload.org, and the other browsers derive theirs from it. Its
`force-https` entries, with their `include_subdomains`, are generated
into a sorted file in `crates/net/data/` by a tool kept in the repo
(as M32's web-platform-tests pull is), read once into a sorted table
the first time a host is asked about, and looked up by the host and
each of its parent domains. The pins, the `expect-ct` entries and the
test domains are left out. Fetching the list at runtime was weighed
and set aside (a request at start that says the browser is running,
and nothing to trust it by); so was leaving the list out (the first
visit to a preloaded site in the clear is what it exists to prevent).
The list ages with the release, as the Public Suffix List does (D20),
and a bump regenerates it.

**`Upgrade-Insecure-Requests`.** Every navigation request, a frame's
included, carries `Upgrade-Insecure-Requests: 1`, as the reference
browser's do (p1). The directive is CSP's, above.

**The mixed form.** A form whose submission would go from a secure
document to an `http:` URL that is not potentially trustworthy (or be
redirected to one) is not sent: the engine shows a page of its own
saying the information would be sent unencrypted, with a "Send anyway"
link that the engine alone follows (it resubmits the same submission,
kept with the page) and a "Go back" link, as the reference browser's
interstitial offers. A submission the user confirms is sent once.

### Local Network Access (M33.4)

**The prompt.** D19 refused outright a request from a public page to a
local address, or from a local page to a loopback one. M33.4 asks the
user instead, as Local Network Access's draft does, when the request's
client is a secure context (an insecure one is still refused at once,
as the draft says) and its document may use the feature: the
`local-network` permission for a local address, `loopback-network` for
a loopback one, by Permissions Policy's default allowlist of `'self'`:
a frame of another origin than the top's is refused unless its
element's `allow` attribute names the feature (or
`local-network-access`, which the reference browser still takes for
both). `allow` is parsed
for these alone; the rest of Permissions Policy is G96.

- *Where*: `net`'s transport still checks the address it is about to
  connect to (D19's rule against DNS rebinding); where it refused, it
  now asks a gate the client was given (`net::LocalNetworkGate`, a
  trait the engine implements), from the fetch's thread, which waits
  for the answer and gives up at the fetch's abort. A response the
  HTTP cache answers is checked the same way (M28's cache answered
  without the check: a hole found for this note). A frame's
  navigation is checked like a subresource, with the parent as
  initiator; a top-level navigation the user starts goes anywhere.
- *The answer*: by origin and permission, kept in the profile once
  given (`granted` or `denied`, as the reference browser keeps a
  site's permissions), asked of the user once while a prompt for the
  same origin and permission is open (the requests that come meanwhile
  wait on the same answer). The engine says a prompt is wanted
  (`Engine::permission_prompt`), the shell shows it under the address
  field ("a.example wants to connect to devices on your local network:
  Allow, Block"), and its answer goes back (`Engine::answer_prompt`).
  A prompt not answered within 60 seconds, or dismissed, denies those
  requests without remembering.
- *Scripts*: `fetch`'s `targetAddressSpace` (`local`, `loopback`;
  `public` does nothing) lets an `http:` request from a secure page to
  a local address past mixed content, failing if the address
  connected to is not in that space, as the draft says;
  `navigator.permissions` is G97 with the Permissions API.

### The profile at rest

**Encrypted by a key the operating system keeps (D27).** M28 left the
profile's file readable by anything that can read the user's files.
The values of the profile, cookies and `localStorage` items, are
encrypted with AES-256-GCM (`aws-lc-rs`, a random 96-bit nonce each
value, the table and key as associated data so a value cannot be
moved to another key) under a 256-bit data key made when the profile
is; the data key is kept in the profile wrapped by the operating
system's user key: on Windows by DPAPI (`CryptProtectData` for the
current user), the call in the shell's platform module, the one
crate that writes `unsafe` (below), handed to `profile` as a pair of
functions. Keys of the tables (origins, a cookie's domain, path and
name) stay readable, as the reference browser's do. A profile of M28
is encrypted in place when opened (its schema moves to 2); a value
that does not decrypt (a profile copied to another user or machine)
is dropped, as the reference browser drops cookies it cannot decrypt.
Alternatives weighed: a password the user types (the reference
browser does not ask one, and a browser that asks at every start is
not used), and leaving it to the disk's encryption (which the user may
not have, and which another program of the user's passes). Where the
operating system offers no such key here (macOS's Keychain and Linux's
Secret Service are not bridged), the data key is kept unwrapped,
which hides the values from a search of the disk and nothing more, as
the reference browser on Linux without a keyring does (G98).

### The memory-safety audit (M33.5)

**`unsafe` in the platform layer alone.** The workspace's own `unsafe`
is one block, the shell's call to `SystemParametersInfoW` (M22), with
its argument written beside it; this milestone adds DPAPI's two calls
there. Every other crate says `#![forbid(unsafe_code)]` (`canvas`,
`js`, `profile` and `svg` already do), the fuzz targets and the test
and bench crates too; the shell says `#![deny(unsafe_code)]` with an
`allow` on its `platform` module alone, every block in it carrying a
`// SAFETY:` argument. The rule is written in `docs/architecture.md`
(T3.2), and the gate and CI's check job grep for `unsafe` outside the
platform module and for a block there without its argument. The
third-party `unsafe` the workspace runs is inventoried in
`docs/security.md`: for each crate of `Cargo.lock`, whether it writes
`unsafe` and why (FFI to C: `aws-lc-sys`, `rquickjs-sys`; the
operating system: `windows-sys`, `winit`, `softbuffer`, `wgpu`'s;
SIMD and performance: `tiny-skia`, `memchr`, `redb`, `wasmi`), how
many blocks, and what it is pinned to, counted by a script over the
registry's sources; and what takes untrusted input among them (the
JavaScript engine, the TLS stack, the image decoders), which is where
M34's sandbox matters.

### Fuzzing graduation (M33.6)

**Targets, corpora, the continuous run.** New targets: `csp` grows to
the whole model (any policy list, any request, inline check and code
check, held to never panicking and to report-only never blocking);
`referrer` (policy parsing and the referrer of any pair of URLs, held
to never leaking more than its policy's part); `hsts` (any field and
any sequence of hosts and answers, held to the store's bound and its
matching); `frames` (pages of frames from `srcdoc`, sandbox tokens,
nested and removed, with messages and navigations between them, the
tree held to its limits and nothing panicking); and `dom_api` and
`pipeline` take frames among their calls and markup. The corpora,
minimized by libFuzzer's merge for each target, are committed under
`fuzz/seeds/<target>/` (T2.2: their size measured when they are made,
kept under 30 MB together, the largest targets' trimmed further by
the merge's `-max_len`), and the gitignored `fuzz/corpus/` grows from
them. CI's fuzz job runs nightly instead of weekly from the committed
seeds and its cache. The OSS-Fuzz application cannot be made: the
service takes public projects, the repository is private, and the
application is the maintainer's to send; what it would hold is written
in `docs/security.md` (G99).

### The threat model and the adversarial site (M33.7, M33.8)

**`docs/security.md`.** Assets (the user's cookies, storage, profile
and credentials, the local network, the machine), adversaries (a web
page and the frames it embeds, a network attacker, a page that
exploits the engine), boundaries and what holds each (origins and
sites, frames, the net crate's egress rule T3.4, the script boundary
and its limits, the decoders and parsers and their bounds, the
profile, and the process boundary M34 brings), trust levels, the
limits table (moved there from `docs/architecture.md`, which said
"until `docs/security.md` (M33)" for them), the `unsafe` inventory, the
fuzz targets, and the known gaps with security weight. T3.3: updated
with every milestone from now on.

**The adversarial site v2.** `crates/engine/tests/security/` gains
pages that attack what this milestone defends, each run by the
engine's tests against victims served by the test server, each
attack's success recorded and required to be none:

- *frames*: framing a victim that sends `X-Frame-Options` or
  `frame-ancestors` (nothing of it shown or loaded beyond the refused
  response); a sandboxed frame reading the parent's cookies and
  storage, navigating the top, submitting a form, opening a popup,
  running a script without `allow-scripts`; reading a frame's
  document, location, name or storage across origins, and a parent's
  from a frame; a message to a frame whose target origin does not
  match;
- *CSP*: injected markup under a strict policy trying an inline
  script, an event handler, a `javascript:` URL, `eval`, `Function`,
  a string `setTimeout`, WebAssembly, a `<style>` and a `style`
  attribute, an image, a `fetch` and a form sent to the attacker's
  host, a `<base>` that takes the page's relative scripts elsewhere, a
  nonce read through an attribute selector, and a frame the policy
  does not allow;
- *cookies and storage*: `HttpOnly` cookies through
  `document.cookie`, `Lax` and `Strict` cookies sent to the victim
  from a cross-site frame, image, form and `fetch`, the victim's
  first-party storage and partitioned cookies read from a third-party
  frame, a `Secure` cookie over `http:`, and a `Referer` that carries
  a secret path across sites;
- *the network*: an `http:` request to a host the profile knows as
  HSTS, and a public page reaching a local address without the user's
  answer.

M27.7's page stays as it is, run with the others.

### Tests and measures

Unit tests in `net` for the policy model, every directive's matching
and fallback, the referrer of every policy and pair of URLs, the HSTS
field, store, preload lookup and upgrade, the partition keys of the
jar, cache and pool, and the gate; in `layout` for the frame's box; in
the engine for each behaviour of this note (frames' loading, drawing,
input, scripting, sandboxing and refusal, each CSP check and
violation, referrers, the prompt) against the test server; in
`profile` for the encryption and the migration. The web-platform
tests: `content-security-policy`, `html/browsers/sandboxing`,
`x-frame-options`, `referrer-policy`,
`html/semantics/embedded-content/the-iframe-element`,
`upgrade-insecure-requests` and `webmessaging`'s frames, taken by the
runner's rule once it serves `.headers` files, fills the `.sub`
templates' hosts and ports (the hosts as names under `localhost`:
`web-platform.localhost` for `{{host}}`, its `www`, `www1` and `www2`
subdomains of the same site, and `not-web-platform.localhost` for the
other site) and runs tests in frames; a test needing a Python handler
(the report collectors among them) stays out. Pages from 139 compared
with the reference browser: frames of every size and border, `srcdoc`,
nesting, a scrolled frame, a sandboxed frame's `<noscript>`. The
adversarial site (above). Benchmarks: `benches/frames.rs` (a page of
twenty `srcdoc` frames loaded and drawn, and scrolled), the other
benches for what the checks cost them (T4.3).

## 2. Not in this milestone, said rather than hidden

Each item that no milestone holds is registered on the G list at the
documents step; the number follows each.

- *Same-origin scripting across frames*: a parent reaching into its own
  origin's frame (`contentDocument`, `contentWindow.document`,
  `parent.someFunction()`), `document.domain` relaxing it, `opener`.
  With M34's site instances sharing a runtime. G90.
- *A frame's layers composited on the GPU*, its pixels drawn in
  software and uploaded. With M34's out-of-process frames. G91.
- *The joint session history*: a frame's navigations as entries of the
  tab's history. G92.
- *Blocking third-party cookies* as a setting. With M37.3's settings.
  G93.
- *Trusted Types* (`require-trusted-types-for`, `trusted-types`). G94.
- *`report-to` and the Reporting API* (`Reporting-Endpoints`), which
  also carries other reports. G95.
- *Permissions Policy* past Local Network Access's features: the
  `Permissions-Policy` field, `allow` for the other features. G96.
- *The Permissions API* (`navigator.permissions.query`). G97.
- *The profile's key on macOS and Linux* (Keychain, Secret Service).
  G98.
- *The OSS-Fuzz application* and continuous fuzzing on its machines,
  for when the repository is public. G99.
- *Revocation and Certificate Transparency*: the reference browser
  checks neither online; it enforces CT and revokes by lists it pushes
  to itself (CRLSets, the CT log list), which go stale without an
  update service: a CT check with a stale list must be switched off,
  as the reference browser switches its own off after 70 days. With
  T6.3's auto-update. G100.
- *Encrypted Client Hello*, which needs a DNS client of the browser's
  own for HTTPS records (DNS over HTTPS). G101.
- *`<frame>`, `<frameset>`, and HTML in `<object>` and `<embed>`*.
  G102.
- *`loading=lazy` frames*, `credentialless`, the `csp` attribute
  (embedded enforcement), `allowfullscreen` and the Fullscreen API.
  G103.
- *Popups*: `window.open`, `_blank` as a new tab, `opener`, and
  `allow-popups-to-escape-sandbox`'s effect. With M37's tabs.
- *Subresource Integrity*, which an external script's hash needs: G43
  already.

## 3. Order of work

Each step a vertical slice, with its tests, a sabotage pass and the
gate.

1. *Frames*: the `iframe` element's child engine and the frame tree,
   `Replaced::Frame` and its user agent rules and hints, the frame
   drawn into the page, `src`, `srcdoc` and the initial `about:blank`
   with their origins, base URLs and policy containers, the `load`
   events and the parent's delayed `load`, removal, the limits,
   `Destination::Iframe`, the frame's request initiated by its parent.
2. *Frames' input*: the pointer, the cursor and links, the wheel and
   scroll chaining, focus and sequential focus, keys, resizing, the
   display's scale, print.
3. *Scripts across frames*: the window proxies and cross-origin
   locations, `length`, `frames`, `parent`, `top` and names,
   `postMessage` between frames, navigating another frame and the
   rule for it, link and form targets.
4. *Partitions*: the site for cookies through ancestors, partitioned
   cookies, storage keys, the cache's key, connections and TLS
   sessions by partition.
5. *Sandboxing and framing refused*: the flags from the attribute,
   the parent and CSP's `sandbox`, what each guards, `X-Frame-Options`
   and `frame-ancestors`, the refused page.
6. *CSP's policies and fetch directives*: the model, report-only,
   every request's check by its destination, nonces and parser
   metadata on requests, `'strict-dynamic'` for scripts, every
   document's policies and their inheritance, workers' own.
7. *CSP inline and code*: scripts by nonce and hash, nonce hiding,
   `<style>` and `style` attributes, event handlers, `javascript:`
   URLs, `eval` and its kin, WebAssembly, `base-uri`, `form-action`,
   `upgrade-insecure-requests`.
8. *CSP's violations*: the event, the console, `report-uri`.
9. *Referrer Policy*.
10. *HSTS*: the field, the store in the profile, the preload list and
    its tool, the upgrade; the `Upgrade-Insecure-Requests` field; the
    mixed form's warning.
11. *Local Network Access's prompt*: the gate, the permissions kept,
    the shell's prompt, `allow`, frames' navigations, the cache's
    answers, `targetAddressSpace`.
12. *The profile encrypted at rest*.
13. *The `unsafe` audit*: the attributes, the rule and its check, the
    inventory.
14. *Fuzzing*: the new targets, the corpora minimized and committed,
    CI nightly.
15. *The web-platform tests*: the runner's headers, templates, hosts
    and frames, the directories above.
16. *Golden cases* from 139.
17. *The adversarial site v2*.
18. *`docs/security.md`*.
19. *Acceptance*: `benches/frames.rs`, T4.3, the fuzz runs.
20. *The documents*.

## 4. Corrected to the code

Where the milestone's code came out otherwise than the sections above
planned, *(corrected)*, by section. The step notes say how each was
found.

*Frames.* *(corrected)* A document given to the engine with frames is
`Loading` until its frames are told. The window's frames by index and
name are a plain object between `Window.prototype` and
`EventTarget.prototype`, made again when the engine's list changes:
the Proxy planned made QuickJS-ng answer `undefined` for a global it
did not know rather than throw. An `iframe` hears `load` for each
document its frame completes, whatever navigated it (its `src`, its
parent's form or link by target, its own script) and even when that
document's own `load` listener navigates on (`Engine::completed`), but
not for a fragment, nor for the initial `about:blank` when its
element's navigation follows; a `src` naming the frame's document with
another fragment goes to the fragment rather than fetching it again,
as the reference browser does (steps 1 and 17).

*Sandboxing and refusing to be framed.* *(corrected)* Several
`X-Frame-Options` values refuse only when `deny`, `allowall` or
`sameorigin` is among them; several others allow, as HTML says. A
frame's first navigation takes its element's sandbox flags as they are
when the frame is made, after the parser's next script; the reference
browser takes them at insertion. A sandbox without `allow-scripts`
styles its document with scripting off too: `noscript` makes boxes and
the `scripting` media feature is `none` (step 16).

*Partitions by top-level site.* *(corrected)* The partition is per
page, not per engine: an incoming document's scripts run while the
shown one is still the initial `about:blank` (`Network::set_partition`).

*Content Security Policy.* *(corrected)* Source lists stay the policy's
tokens, each matched by its kind when a check needs it, rather than
expressions parsed once. More than 64 report-only policies refuse
everything (`MAX_POLICIES`), failing closed. A form a document submits
while it is still parsing is held to that document's `form-action`
and sent with its body (step 17).

*Referrer Policy.* *(corrected)* `document.referrer` is the referrer
the first hop sent; a stylesheet's own requests carry the document's
URL, and a worker's its own by the default policy, its response's
field not read (registered).

*HSTS and Upgrade-Insecure-Requests.* *(corrected)* The preload list's
file ships empty, the reference browser's list waiting for the
maintainer's leave to fetch it, and is read into a hash map; a
field's `max-age` counts for a year at most; a form submitted into
another frame by its target, or redirected to `http:`, is not held on
the warning page (registered).

*Local Network Access.* *(corrected)* An `allow` attribute's
allowlist is not matched against the frame's origin, which Permissions
Policy (G96) would do.

*The profile at rest.* *(corrected)* A cookie's record is sealed whole,
its domain, path and name with it; pages `redb` freed when a profile
of schema 1 was sealed keep their values in the clear until written
again; opening a profile with a wrap that cannot unwrap its key
replaces the key.

*The memory-safety audit.* *(corrected)* M29's WebAssembly fixture
(`crates/engine/tests/wasm/numbers/lib.rs`), a crate built to
WebAssembly whose exports `#[unsafe(no_mangle)]` marks, is left out of
the check, which cuts strings before it reads.

*Fuzzing graduation.* *(corrected)* The seeds are cut to 1,024 bytes
for six targets to keep them under 30 MB together, and marked binary.
Their merge found what the runs had not: a realm with no document
refusing code from strings, and the differential comparison naming a
frame's box by node number, missing canvases' bitmaps and drawing the
fresh engine's frames unloaded; and a canvas a script resized not laid
out again, since M30.

*The adversarial site.* *(corrected)* The reads across frames fail
with the defences taken away too, every frame being of an origin of
its own (G90); the test names them, to succeed once that closes.

*Tests and measures.* *(corrected)* `x-frame-options` and
`upgrade-insecure-requests` give no web-platform test the runner can
serve, and are not pulled; two racing tests are left out by name.
The golden pages have no frame scrolled by its own script, which the
engine cannot do (G46); 3D borders and the viewport's overflow came
with them (step 16).
