# Design note: fetch, the same-origin policy, CORS and cookies

Roadmap M27. Written before the code, as the record of the decisions,
and corrected to the code when the milestone ended: where they
disagreed, the code won, and the places are marked *(corrected)*. What
ships is described in `docs/architecture.md` "Current state".

A page's scripts can change the page but cannot talk to a server. There
is no `fetch`, no `XMLHttpRequest`, no `FormData`, no
`URLSearchParams`; `document.cookie` does not exist and
`navigator.cookieEnabled` says `false`, because no cookie is ever kept
or sent. `net` sends `GET`, `HEAD` and `POST` and nothing else, with
headers it chooses itself; a request has no credentials mode, no
origin, no redirect mode and no idea of CORS. The engine keeps no
origin for a document: it computes one from the URL when it needs one,
and a `file:`, `data:` or `about:blank` document gets a new opaque
origin at every asking, so it is not even same-origin with itself. The
one cross-origin rule the engine has, a script from another origin
having its errors muted, says in its comment "no CORS until M27".

This milestone gives scripts the network and puts the walls around it:
origins kept and compared, sites through the Public Suffix List, the
same-origin policy stated vector by vector, `fetch()` with its
`Headers`, `Request` and `Response`, the bodies scripts send
(`FormData`, `URLSearchParams`, `Blob`), CORS with its preflights,
cookies as RFC 6265bis has them, mixed content for what scripts fetch,
and `XMLHttpRequest`. Its acceptance is an adversarial page that tries
every way of reading across origins that this engine offers, and fails
at each; it stays in the repository as a regression test.

The standards are read at the versions current when this note was
written:

- the Fetch Standard (Living Standard of 6 October 2026): 2.2 (methods,
  headers and their lists, requests and responses, filtered
  responses), 2.9 (bad ports), 3.1 (cookies' hooks), 3.2 (`Origin`),
  3.3 (the CORS protocol), 3.7 (`Cross-Origin-Resource-Policy`), 4.1
  to 4.10 (main fetch, scheme fetch, HTTP fetch, HTTP-redirect fetch,
  HTTP-network-or-cache fetch, the CORS-preflight fetch and its cache,
  the CORS check), 5 (`Headers`, `BodyInit`, the `Body` mixin,
  `Request`, `Response`, `fetch()`);
- Cookies: HTTP State Management Mechanism,
  draft-ietf-httpbis-rfc6265bis-22 (1 December 2025, with the RFC
  Editor): 4.1.3 (the prefixes), 5.2 (same-site and cross-site
  requests), 5.4 to 5.8 (the user agent's side: the `Set-Cookie`
  parsing, the storage model, the `SameSite` enforcement, retrieval);
- the XMLHttpRequest Standard (Living Standard of 18 August 2026): 3
  (the interface, its states, `open()`, `send()`, the response and its
  types), 4 (`FormData`), 5 (`ProgressEvent`);
- the HTML Standard (Living Standard of 9 October 2026): 7.1.1 (origins,
  sites, `document.domain`), 8.1.3.5 (secure contexts), 3.1.4
  (`document.cookie`), 2.5.4 (CORS settings attributes), 4.10.22.4
  (constructing the entry list, for `FormData`);
- the URL Standard (Living Standard of 10 September 2026): 6
  (`URLSearchParams`) and 5 (`application/x-www-form-urlencoded`);
- the File API (W3C Working Draft): `Blob` and `File`;
- Mixed Content (Editor's Draft of 23 February 2023) and Secure Contexts
  (potentially trustworthy URLs);
- Fetch Metadata Request Headers (`Sec-Fetch-Dest`, `-Mode`, `-Site`,
  `-User`).

What the standards leave to the implementation, or state but are worth
checking, was put to the reference browser (Chrome 154.0.8037.99,
headless) before this note was written, from pages served on two ports
of 127.0.0.1 and on `localhost` (scratchpad `m27/probe_server.py`,
`pages/p1.html` to `p4.html`):

- `fetch` rejects with `TypeError: Failed to fetch` for a network
  error, a CORS failure, `about:blank`, an unknown scheme and
  `redirect: "error"` alike; a URL that does not parse, a forbidden
  method (`CONNECT`) and a URL with credentials throw at once, also as
  `TypeError`; an abort rejects with the signal's reason,
  `AbortError: signal is aborted without reason` by default;
- a same-origin `GET` sends no `Origin`; a `POST` sends it, and so does
  every cross-origin request; after a redirect that crossed origins it
  is `null`; `Sec-Fetch-Site`, `-Mode` and `-Dest` go with each
  request, `same-origin`, `cors`, `empty` for a `fetch`;
- methods are normalized only for `DELETE`, `GET`, `HEAD`, `OPTIONS`,
  `POST` and `PUT`: `patch` goes out as `patch`; a preflight's
  `Access-Control-Allow-Methods: delete` does not allow `DELETE`;
- a CORS response without `Access-Control-Expose-Headers` shows
  `content-length` and `content-type` only; `*` shows everything but
  `Set-Cookie`; two `Access-Control-Allow-Origin` values fail; a
  credentialed request fails with `*` and needs the origin and
  `Access-Control-Allow-Credentials: true`;
- a preflight is sent for `PUT`, `PATCH`, `DELETE`, a header outside
  the safelist and `Content-Type: application/json`, with
  `Access-Control-Request-Headers` lower-cased, sorted and joined by
  commas without spaces (`content-type,x-b`); with no
  `Access-Control-Max-Age` it is still cached for the next identical
  request moments later (Fetch's default of 5 seconds);
- a redirect answered without `Access-Control-Allow-Origin` fails a CORS
  request; `redirect: "manual"` gives a response of type
  `opaqueredirect`, status 0, no headers; 301 and 302 turn a `POST` into
  a `GET` without its body, 303 too, 307 keeps it;
- `Headers` iterates its names lower-cased and sorted, values of one
  name joined by `, `, `set-cookie` kept apart, one pair a value; a
  request's forbidden headers (`Cookie`, `Origin`, `Host`, `Sec-*`) are
  dropped silently; `no-cors` keeps only the safelisted ones and
  refuses `PUT`;
- a `Response` made by a script is of type `default`, its
  `content-type` `text/plain;charset=UTF-8` for a string; a body read
  twice throws `TypeError` ("body stream already read"); a 204 cannot
  have one; a status outside 200 to 599 is a `RangeError`;
- `FormData` sent gives `multipart/form-data; boundary=...`,
  `URLSearchParams` `application/x-www-form-urlencoded;charset=UTF-8`,
  a string `text/plain;charset=UTF-8`;
- `self.origin` and `location.origin` agree; `isSecureContext` is true
  on `http://127.0.0.1`, and on a `file:` page, whose origin is `null`,
  whose `document.cookie` is empty and ignores what is set;
- cookies are kept by host, not by port: a cookie set by
  `127.0.0.1:8032` is sent to `127.0.0.1:8031` and read there; the
  `Cookie` header orders longer paths first, then older cookies;
  `HttpOnly` cookies are sent but not read, and a script cannot set
  one; `Secure` is accepted on `http://127.0.0.1`; `__Host-` needs
  `Secure`, `Path=/` and no `Domain`, `__Secure-` needs `Secure`;
  `Domain=127.0.0.1` on that host is accepted, `Domain=0.1` is not;
  4,096 bytes of name and value are accepted; a nameless cookie
  (`document.cookie = "=v2"`) reads as `v2`, and replaces `novalue`;
- across sites (`127.0.0.1` to `localhost`), with credentials, only a
  cookie with `SameSite=None; Secure` is sent; one with no `SameSite`,
  `Lax` or `Strict` is not; `SameSite=None` without `Secure` is not
  kept; a response to a request without credentials sets nothing;
- `XMLHttpRequest`: `readystatechange` 1 at `open`; `send` fires
  `loadstart` before it returns, the upload's `loadstart` too when it
  has a body; then `readystatechange` 2 and 3, one `progress`
  (`loaded` equal to `total`), `readystatechange` 4, `load`, `loadend`;
  a network or CORS error: `readystatechange` 4, `error`, `loadend`,
  status 0; a synchronous request fires `readystatechange` 4 and `load`
  before `send` returns, throws `NetworkError` on failure, and refuses
  `timeout` and `responseType` (`InvalidAccessError`);
  `getAllResponseHeaders` is lower-cased, sorted and combined, each line
  ending in CRLF.

## 1. Decisions

**A document keeps its origin (M27.2).** `Page` gains an `origin`,
fixed when the page is made, and every check reads it: the same-origin
tests that exist (an image's orientation, a script's muted errors,
`location.origin`) and every one this milestone adds.

- A document fetched over `http:` or `https:` has its final URL's tuple
  origin.
- A `file:`, `data:` or fetched `about:blank` document, a page the
  browser makes (an error page, the resubmission page) and a document
  given to the engine get a new opaque origin, made once, so that the
  document is same-origin with itself and with nothing else. Chrome's
  `file:` origin serializes as `null` and so does ours.
- The initial `about:blank` has an opaque origin as well: HTML gives it
  its creator's, and it has none.
- *A secure context* is a document whose URL is potentially
  trustworthy (Secure Contexts 3.1): `https:`, `wss:`, `file:`, a
  loopback address, `localhost` and names under it, and `about:blank`
  and `about:srcdoc`; `isSecureContext` reports it. `net::url` gains
  `Url::is_potentially_trustworthy`, which the mixed-content check in
  `net::fetch` used privately until now.
- `self.origin` (`window.origin`), `isSecureContext` and
  `document.domain` join the window and the document. `document.domain`
  reads the origin's host, or the empty string for an opaque origin;
  setting it validates as HTML says (a `SecurityError` for an opaque
  origin, or a value that is not the host or a registrable suffix of
  it) and then changes nothing, as HTML does in an origin-keyed agent
  cluster, which the reference browser's documents have been by default
  since 2023.

**Sites, and the Public Suffix List (D20).** HTML's site is an opaque
origin, or a scheme and the host's registrable domain; cookies'
`Domain` attribute and their same-site rules, and `Sec-Fetch-Site`,
need it. The registrable domain needs the Public Suffix List. The
choice, recorded as D20: the `psl` crate (2.1.241, MIT or Apache-2.0,
`no_std`, `forbid(unsafe_code)`), a matcher generated from the list
with its ICANN and private sections, rather than the list vendored with
a matcher of our own. It is data, not engine code (roadmap 1.3), and
the list moves weekly: a bump of the crate keeps it current, where a
vendored copy needs a tool and a downloaded file. `net::url` gains
`Host::registrable_domain`, `Host::public_suffix`, `Origin::site` and
`same_site` (schemeful, as the reference browser and HTML's "same site"
are). An IP address or an opaque host is its own site; a host the list
does not know falls to its default rule, `*`, so `a.localhost` and
`b.localhost` are different sites, which the tests use.

**The same-origin policy, vector by vector (M27.2).** There is one
document per tab and no frames (M33.2 brings them), so a script never
holds another origin's window or document: the DOM's vector is closed
by construction. What remains is what the network gives a page:

| Vector | Rule | Where it is enforced |
|---|---|---|
| A response read by `fetch` or XHR | its body, status and headers only when same-origin, or CORS-approved; otherwise a network error, or an opaque response with none of them | `net::fetch` (tainting, the CORS check), the engine handing scripts only `Response::exposed` |
| The URL a request ended at | not shown for an opaque response; a `manual` redirect's target never shown | `net::fetch` |
| A request a page sends elsewhere | allowed when it is one a form could send (a CORS-safelisted method and headers, no credentials beyond the cookies' rules); otherwise preflighted | `net::fetch` (the preflight) |
| Cookies | sent by the jar's rules (domain, path, `Secure`, `SameSite`, the credentials mode); `HttpOnly` never read by scripts | `net::cookies` |
| A script from another origin | runs; its errors muted unless fetched with CORS (`crossorigin`) | the engine (M24, now by tainting) |
| A stylesheet or image from another origin | used; nothing of it readable by scripts (no CSSOM rules, no canvas yet) | nothing to read until M30 |
| A response's own refusal | `Cross-Origin-Resource-Policy` on a `no-cors` response | `net::fetch` |
| A public page reaching the local network | refused (D19, M14) | `net::transport` |

**A request, as `net` takes it (M27.1, M27.3).** `net::Request` becomes
Fetch's request in the parts this milestone needs. Its new fields:

- `method`: any token, no longer three variants. `Method` keeps `GET`,
  `HEAD` and `POST` as constants and gains `Method::parse`, which
  refuses what is not a token and normalizes the six names Fetch
  normalizes; `CONNECT`, `TRACE` and `TRACK` are forbidden for scripts.
- `headers`: the author's header list, which `net` merges with its own
  (a script's `Accept` or `User-Agent` replaces `net`'s, as Fetch
  allows; `User-Agent` is no longer a forbidden name in Fetch).
- `mode`: `Navigate`, `SameOrigin`, `NoCors` or `Cors`.
- `credentials`: `Omit`, `SameOrigin` or `Include`.
- `redirect`: `Follow`, `Error` or `Manual`.
- `origin`: the request's origin, the document's for a script's
  request, used for `Origin`, CORS and the tainted-origin flag.
- `client`: the document's URL and site, for cookies' same-site rules
  and `Sec-Fetch-Site`, which navigations now carry too: a link or a
  form's navigation is initiated by its document even though it is not
  subject to the subresource checks `initiator` triggers.
  *(corrected)* No field of its own: with no frames a document is its
  own top level, so the request's `origin` gives the site for cookies
  and `Sec-Fetch-Site`; a navigation a document starts carries its
  origin (`Request::from_document`) without an initiator.
- `destination` gains `Empty`, a `fetch`'s and an XHR's, whose `Accept`
  is `*/*`.
- `user_activated`, for `Sec-Fetch-User`. *(corrected)* Set for a
  navigation the user started: typed, a link clicked, a submit
  button's activation; a script's `form.submit()` or `location` is
  none, the engine telling no script inside a user's click from one
  that is not.
- `preflight`: Fetch's use-CORS-preflight flag, which XHR sets when its
  upload has listeners.

**What a request sends, now.** To M14's list (`Host`, `User-Agent`,
`Accept`, `Accept-Language`, `Accept-Encoding`, `Content-Type` with a
body):

- `Origin` (Fetch 3.2): on a CORS request, and on any request whose
  method is neither `GET` nor `HEAD`, navigations included (a form's
  `POST`); `null` once a redirect has crossed origins;
- `Cookie` (below), by the credentials mode and the jar's rules;
- `Sec-Fetch-Dest`, `Sec-Fetch-Mode`, `Sec-Fetch-Site` and, for a
  navigation the user started or activated, `Sec-Fetch-User: ?1`, to
  potentially trustworthy URLs only, as Fetch Metadata says;
- the author's headers, for a script's request.

Still not sent: `Referer` (M33.3).

**Headers' lists (`net::headers`).** Fetch's lists and tests become
functions there, used by `net` itself and by the engine when it checks
what a realm asks for: a header name (a token) and value (no NUL, CR or
LF, HTTP whitespace trimmed); the forbidden request-header names and
methods-override check; the CORS-safelisted request headers (with the
128-byte value and 1,024-byte total limits) and CORS-unsafe names;
the no-CORS-safelisted names; the forbidden response-header names
(`Set-Cookie`, `Set-Cookie2`); the CORS-safelisted response-header
names; `Access-Control-Expose-Headers` and `-Allow-Headers` parsed as
lists; and "sort and combine", which `Headers` iterates by.

**Redirects (Fetch 4.5).** `net` keeps following them in its own loop,
now with the request's redirect mode: `Error` makes a redirect a
network error, `Manual` stops at it with an opaque-redirect response
(status 0, no headers, no body; its location never reaches a script).
On each hop, beyond M14's checks: a redirect to a URL with credentials
is a network error for a CORS request; a CORS request checks the
redirect response itself (its `Access-Control-Allow-Origin`), as the
reference browser does; when a hop crosses origins the request's
origin becomes tainted (`Origin: null` from then on) and a request
that was same-origin is checked by CORS from that hop on (or made
opaque, in `no-cors` mode); a method changed to `GET` drops the
body and the request-body headers (`Content-Type`, `Content-Encoding`,
`Content-Language`, `Content-Location`); `Set-Cookie` on every hop is
stored.

**CORS (M27.3).** In `net::fetch`, as main fetch says:

- *Tainting.* A request's response tainting is `basic` while the
  request is same-origin (or `data:`), `cors` for a `Cors` request to
  another origin, `opaque` for a `NoCors` one; `SameOrigin` mode to
  another origin is a network error.
- *The preflight* (4.8): an `OPTIONS` request with
  `Access-Control-Request-Method` and, when there are CORS-unsafe
  headers, `Access-Control-Request-Headers` (lower-cased, sorted,
  comma-joined), no credentials, no author headers; it succeeds on a
  status from 200 to 299 that passes the CORS check and allows the
  method (exactly, or by `*` without credentials; the safelisted ones
  always) and every unsafe header (by name, or `*` without
  credentials; never `Authorization` by `*`). A failure is a network
  error.
- *The preflight cache* (4.9): one per `Client`, keyed by the request's
  origin, URL, credentials and method or header name, each entry
  living `Access-Control-Max-Age` seconds, 5 when absent
  (`DEFAULT_PREFLIGHT_AGE`), at most 7,200 (`MAX_PREFLIGHT_AGE`, the
  reference browser's cap), at most 1,024 entries
  (`MAX_PREFLIGHT_ENTRIES`, the oldest going first).
- *The CORS check* (4.10): exactly one `Access-Control-Allow-Origin`,
  equal to the serialized origin, or `*` without credentials; with
  credentials, `Access-Control-Allow-Credentials: true`.
- *Errors* say nothing to the page: every failure is the same network
  error, whatever the server answered. *(corrected)* The console says
  why (`Failed to fetch URL: why`, for `fetch` and XHR, step 8), where
  only the user reads it.

**What a script sees of a response.** `net::fetch::Response` keeps the
whole response (the engine needs an opaque image's or script's body)
and gains its `tainting`; `Response::exposed()` is the only view the
engine gives a realm, Fetch's filtered response:

- `basic`: everything but `Set-Cookie` and `Set-Cookie2`;
- `cors`: the status, the CORS-safelisted response headers and those
  `Access-Control-Expose-Headers` names (`*` for all, without
  credentials), the body;
- `opaque` and `opaqueredirect`: type, status 0, no status text, no
  headers, no URL, no body. *(corrected)* An `opaqueredirect` keeps its
  URL list, as Fetch's filtered response does: its `url` is the
  request's, never where the redirect points.

The body of an opaque response never enters a realm: the conversion is
one function, in `net`, tested there, and the engine's fetch path calls
nothing else. *`Cross-Origin-Resource-Policy`* (3.7) is checked on
`no-cors` responses from other origins: `same-origin` and `same-site`
refuse them, by a network error, as M14's note promised for M27.

**The CORS settings attribute.** `crossorigin` on `script`, `link
rel=stylesheet` and `img` makes their requests `Cors` (credentials
`SameOrigin` for `anonymous` and the empty value, `Include` for
`use-credentials`); a CORS failure is the element's `error`. A script
fetched that way has its errors unmuted when the CORS check passed: the
muting reads the response's tainting, no longer whether every URL was
same-origin. An image with `crossorigin` from an `https:` page to
`http:` is blocked rather than upgraded (Mixed Content 3).
*(corrected)* An `img`'s `crossorigin` is left for canvas (M30), the
only reader its CORS mode changes: the image store keys images by URL
alone. `net` blocks a CORS-mode image's `http:` request from an
`https:` page rather than upgrading it (step 8), though no image asks
so until then.

**Cookies (M27.4).** RFC 6265bis-22's user agent, in a new module,
`net::cookies`, with the jar inside `Client`, so that every fetch, its
redirects included, reads and writes the same one. *(corrected)* The
jar is the engine's (`Network` keeps it) and every client it makes is
given it (`Client::with_cookies`), so that a client made again, D12's
switch, keeps the cookies; a document's own `Set-Cookie` is stored
when its head arrives, so its first script reads it:

- *One jar per client.* The roadmap's "jar per-origin" reads as the
  RFC's storage: cookies are kept by domain and path, and a host's
  cookies are shared by its ports and, for `Domain` cookies, its
  subdomains, as the reference browser showed. An engine has one
  client, so one jar; its tests each make their own. Nothing is
  written to disk: persistence is M28's.
- *Parsing* (5.6): the `Set-Cookie` value split at the first `;`,
  name and value at the first `=`, a pair without `=` a nameless cookie
  (empty name), both trimmed; a cookie with a control character other
  than a tab, or with an empty name and value, or a name and value
  longer than 4,096 bytes together (`MAX_COOKIE_SIZE`), is ignored; an
  attribute value longer than 1,024 bytes (`MAX_ATTRIBUTE_SIZE`) is
  ignored; `Expires` by the RFC's date algorithm, `Max-Age` winning,
  both capped at 400 days (`MAX_COOKIE_AGE`); `Domain`, `Path`,
  `Secure`, `HttpOnly`, `SameSite` (`Strict`, `Lax`, `None`, anything
  else as absent).
- *Storing* (5.7): a `Domain` that is a public suffix is ignored when it
  is the host itself (a host-only cookie) and refuses the cookie
  otherwise; a `Domain` the host does not domain-match refuses it; an
  IP host takes a `Domain` only equal to itself. `Secure` is accepted
  only from a potentially trustworthy URL (the reference browser keeps
  `Secure` from `http://127.0.0.1`); a cookie from an insecure URL may
  not shadow a `Secure` one of the same name whose domain and path
  overlap it. The prefixes, matched without regard to case: `__Secure-`
  needs `Secure`; `__Host-` needs `Secure`, `Path=/` and no `Domain`. A
  nameless cookie whose value starts with either prefix is refused.
  `SameSite=None` needs `Secure`. A cookie set from a cross-site
  request with `SameSite` other than `None` is refused, and so is any
  cookie of a response to a request that sends no credentials. A script
  (the non-HTTP API) cannot set or replace an `HttpOnly` cookie. The
  same name, domain and path replaces, keeping the creation time; an
  expired cookie deletes.
- *Limits*: 180 cookies a domain (`MAX_COOKIES_PER_DOMAIN`, the
  reference browser's) and 3,000 in the jar (`MAX_COOKIES`), the RFC's
  floors being 50 and 3,000; past either, the expired go first, then
  the least recently used.
- *Retrieval* (5.8): domain and path matched, `Secure` only to a
  potentially trustworthy URL, `HttpOnly` only to HTTP, then
  `SameSite`; ordered by longer path, then earlier creation; joined by
  `; `, a nameless cookie as its value alone.
- *Same-site* (5.2): a request is same-site when its client's site is
  same-site with the URL (schemeful). A navigation the user started
  (no client) is same-site. `Strict` cookies go only with same-site
  requests; `Lax` ones also with a cross-site top-level navigation by a
  safe method (`GET`, `HEAD`); `None` with any. A cookie without
  `SameSite` is `Lax`, as the roadmap and the reference browser have
  it. The RFC's "Lax-allowing-unsafe" (a recent cookie sent with a
  cross-site top-level `POST`) is not taken: it is optional, and its
  window was a migration aid.
- *Credentials*: `Omit` sends and keeps none; `SameOrigin` only for a
  same-origin URL (with the request's origin not tainted); `Include`
  always, subject to the rules above. Navigations, subresources and
  `<img>` include them (`no-cors` defaults to `Include`).
- *`document.cookie`* (HTML 3.1.4): the empty string, and setting
  ignored, in a cookie-averse document (one whose URL is not `http:` or
  `https:`, as the reference browser's `file:` page showed); a
  `SecurityError` for any other document with an opaque origin;
  otherwise the jar's cookies for the URL by the non-HTTP API, as a
  same-site request (the document is its own top level), and setting
  stores one cookie by that API. `navigator.cookieEnabled` becomes
  `true`. *(corrected)* A `data:` document is cookie-averse too, and
  the `SecurityError` needs a sandboxed frame, which this engine has
  not: no document meets it.
- *Third-party cookies* are as the reference browser has them by
  default: `SameSite=None; Secure` cookies are sent across sites. A
  policy of blocking or partitioning them is M33's, with privacy.

**Mixed content (M27.5).** An `https:` document's requests to `http:`
URLs that are not potentially trustworthy are blocked, on every hop:
since M14 for stylesheets, since M23 for images (upgraded instead), and
the check already applies to anything with an initiator; this milestone
makes `fetch` and XHR requests carry the document as initiator, so
they are blocked as blockable content, with a network error the page
sees as `TypeError` or an XHR `error`. A CORS-mode image is blocked,
not upgraded *(corrected: in `net`; no image asks so until M30)*. A
navigation is not mixed content. The form submission
from an `https:` page to `http:` that the reference browser interrupts
with a warning is M33's, with Upgrade-Insecure-Requests.

**`fetch()` and its classes (M27.1).** In the bootstrap, as Fetch's IDL
has them:

- `Headers` with its guards (`immutable`, `request`, `request-no-cors`,
  `response`, `none`), `append`, `delete`, `get`, `getSetCookie`,
  `has`, `set` and iteration by sort-and-combine;
- `Request` (URL resolved against the document's base URL, a URL with
  credentials refused, `method`, `headers`, `body`, `mode`,
  `credentials`, `redirect`, `signal`, and the members that only
  report, `cache`, `destination`, `referrer` (`about:client`),
  `referrerPolicy`, `integrity`, `keepalive`, `duplex`), `clone`;
  *(corrected)* `priority` is checked (`high`, `low`, `auto`) and kept
  by nothing;
- `Response` (constructor, `error()`, `redirect()`, `json()`, `type`,
  `url`, `redirected`, `status`, `ok`, `statusText`, `headers`,
  `clone`);
- the `Body` mixin: `bodyUsed`, `arrayBuffer`, `blob`, `bytes`,
  `formData`, `json`, `text`;
- `fetch(input, init)`, which settles its promise from the engine's
  answer, and rejects at once on an aborted signal or when the signal
  aborts, telling the engine to abort the request.

*Bodies are whole (full-buffer first).* A response's body is read to
its end by `net` before the engine hands it over, and a request's body
is made whole before it is sent. There is no `ReadableStream`:
`Response.prototype.body` and `Request.prototype.body` are not defined,
so a script that tests for them finds them absent rather than a broken
stream. Streams are G67.

*Schemes, as the reference browser has them*: `http:`, `https:` and
`data:` (a `data:` response is `basic`, from any page); `about:`,
`blob:` and anything else a network error; `file:` a network error
even from a `file:` page, as in the reference browser, since a local
page reading other local files is the oldest of these attacks
(stylesheets and images from a `file:` page keep loading, M14).
*(corrected)* A response to `HEAD` has no body, whatever its scheme
gave (Fetch's main fetch; a `data:` URL's did, step 11).

**Bodies scripts make.** `Blob` and `File` (constructor from strings,
buffers, views and blobs, with `type` and line endings `transparent` or
`native`; `size`, `type`, `slice`, `text`, `arrayBuffer`, `bytes`;
`File`'s `name` and `lastModified`), `FormData` (from a form, M26's
entry list through a native; `append`, `delete`, `get`, `getAll`,
`has`, `set`, iteration; a file value a `File`) and `URLSearchParams`
(from a string, pairs or a record; `append`, `delete`, `get`,
`getAll`, `has`, `set`, `sort`, `size`, iteration, `toString`, the
URL Standard's urlencoded parser and serializer, which `net::url::form`
gains). As bodies: a string is `text/plain;charset=UTF-8`,
`URLSearchParams` `application/x-www-form-urlencoded;charset=UTF-8`,
`FormData` `multipart/form-data` with M26's encoder and a boundary of
ours, a `Blob` its type when it has one, a buffer or view nothing.
`URL` itself, with its setters, is G68 (done in gap batch A);
`TextEncoder` and `TextDecoder` are G69 (done in gap batch A too).
*(corrected)* Web IDL's conversions are taken in its order (found by
the web-platform tests, step 11): a sequence's values converted as the
iteration reaches them, its iterator read once; a record's keys by
their own property keys, a `Symbol` failing, a repeated key's later
value kept; `Blob`'s arguments in order, its `slice` bounds rounded as
`[Clamp]` has them.
HTML's `formdata` event
is not fired, as M26 did not, and with it the constructing entry list's
`InvalidStateError` (G76). `FileReader` is not given (G75; done in
gap batch A).

**The realm asks, the engine decides.** A script's request crosses
into Rust as plain data (`Call::Fetch`: an id, the method, URL,
header pairs, body bytes and the modes), and the engine checks all of
it again whatever the bootstrap did: the URL parsed against the
document, the method a token and not forbidden, every header valid,
forbidden names dropped, `no-cors` restricted, the origin, client and
initiator the document's own. A page that rewrites the bootstrap's
classes reaches the same checks. The engine starts the fetch on its
pool like any subresource (`Target::Fetch { id }`), keyed to the
document so that leaving the page aborts it. *(corrected)*
`Target::Fetch` holds no id: the network's id of the fetch is kept
with the script's number in `Scripts::fetching`; the client is the
request's origin, as above.

- *Answers are tasks.* When a fetch completes, its exposed response
  becomes a task in the document's queue (the networking task source),
  runnable from when it arrived and run by `run_tasks` with the timers
  in the order they became runnable, followed by a microtask
  checkpoint; `next_task_at` reports it. The realm gets it through a new
  `js::Task::Fetched { id, response }`, whose entry resolves or rejects
  the promise the bootstrap keeps by id, as it keeps timers.
  *(corrected)* Answers that came at one time, as a hand's clock gives,
  run in the order they were asked for (step 10); a document on its
  way hears its answers as it does its timers.
- *Bytes cross the boundary* as `ArrayBuffer`s both ways: a native
  argument decoder for buffers and views, and the task's body made an
  `ArrayBuffer` in the realm. Strings in the bodies are UTF-8 encoded
  by the bootstrap's own encoder (no `TextEncoder` is exposed).
  *(corrected)* From the realm to the engine as a string of code units
  0 to 255: `rquickjs` reads a buffer's bytes only through `unsafe`,
  which `js` does not write. From the engine as an `ArrayBuffer`
  (`ArrayBuffer::new_copy`).
- *A document kept by the back-forward cache* keeps nothing in flight:
  as now, a page left with a fetch running is not kept.

**`XMLHttpRequest` (M27.6).** In the bootstrap on the same native fetch,
with `XMLHttpRequestEventTarget`, `XMLHttpRequestUpload` and
`ProgressEvent`:

- the states and events as the standard and the probe have them, one
  `progress` (and one upload `progress`) since bodies are whole;
- `open` (method and URL checked, `async` false allowed),
  `setRequestHeader` (forbidden names ignored, repeated names combined),
  `timeout` (a timer of the realm's that aborts and fires `timeout`),
  `withCredentials` (`Include` when true, `SameOrigin` otherwise),
  `upload`, `send` (the body types above, and a `Document`, serialized,
  is G70's), `abort`, `responseURL`, `status`, `statusText`,
  `getResponseHeader`, `getAllResponseHeaders`, `overrideMimeType`,
  `responseType` (`""`, `text`, `json`, `arraybuffer`, `blob`;
  `document` is G70), `response`, `responseText`; text decoded as UTF-8
  unless the MIME type names another encoding, which the `encoding`
  crate decodes;
- *synchronous requests* (deprecated, still used): `Call::FetchNow`,
  which the host answers by fetching on the engine's thread through the
  client it is now lent, blocking the page as the reference browser
  does; `net`'s connect and read timeouts bound it.

*(corrected)* The upload's events follow the standard, fired only when
the upload had listeners at `send()`; the reference browser also fires
the upload's `loadend` for an aborted `GET`. `ProgressEvent`'s `loaded`
and `total` are doubles, as XHR has had them since 2024, and every
event constructor's length is 1. `open()` converts its user and
password but uses neither (G71); a URL with credentials ends in a
network error.

**Limits (the bounds a hostile page cannot pass).**

| Constant | Value | Bounds |
|---|---|---|
| `MAX_COOKIE_SIZE` | 4,096 bytes | a cookie's name and value |
| `MAX_ATTRIBUTE_SIZE` | 1,024 bytes | a cookie attribute's value |
| `MAX_COOKIE_AGE` | 400 days | a cookie's life |
| `MAX_COOKIES_PER_DOMAIN` | 180 | a domain's cookies |
| `MAX_COOKIES` | 3,000 | the jar |
| `DEFAULT_PREFLIGHT_AGE` | 5 s | a preflight's cached life without `Max-Age` |
| `MAX_PREFLIGHT_AGE` | 7,200 s | a preflight's cached life |
| `MAX_PREFLIGHT_ENTRIES` | 1,024 | the preflight cache |
| `MAX_SCRIPT_FETCHES` | 1,000 | a document's requests in flight (more reject) |
| `MAX_REQUEST_BODY` | 64 MiB | a script's request body |
| `MAX_REQUEST_HEADERS` | 256 KiB | a script's header list |

A response's body stays bounded by `net`'s 64 MiB; the realm's heap by
its 256 MiB.

**Tests.** `net`'s unit tests use two test servers and the names under
`localhost`, which resolve to loopback without the system resolver:
`a.localhost` and `b.localhost` are two sites, two ports of
`127.0.0.1` two origins of one site, so every cookie and CORS rule is
tested on this machine. The engine's tests run pages that fetch from
such servers, on a hand clock. The adversarial page (M27.7) lives in
`crates/engine/tests/security/`: served from one origin, it tries every
vector of the table above against a second origin that holds a secret
in a body, a header, a status, a redirect's location, an `HttpOnly`
cookie and a `Lax` cookie, and writes what it learned; the test
asserts it learned nothing, and that the same reads succeed once the
second origin opts in through CORS, so that the page cannot pass by
failing at everything. *(corrected)* Served from `a.localhost`
against `b.localhost`, it makes 26 reads (`fetch` in its modes, XHR,
script errors, a stylesheet read back, `document.cookie`); against an
origin that allows it, it learns by exactly the 11 CORS reads, and an
unexposed field stays hidden. The reference browser, given the same
page and servers, learned the same.

**Fuzzing.** A new target, `cookies`: a `Set-Cookie` value, a URL pair
and a script's string fed to the jar, which must never keep a cookie
the rules refuse (checked independently on what it returns), never
exceed its limits, and serialize what it keeps to a `Cookie` header
that parses back to the same pairs. `dom_api` gains `fetch`, XHR,
`Headers`, `FormData`, `URLSearchParams`, `Blob` and `document.cookie`
calls, against `data:` URLs on its hand clock. *(corrected)* The
header is compared as serialized: a nameless cookie whose value holds
`=` serializes as a named pair, which no reading tells apart.
`dom_api`'s answers are taken after each run once no fetch is in
flight (`differential::settle_network`).

**The web-platform tests.** The harness runs reftests, check-layout
tests and crashtests; Fetch's and XHR's tests are testharness.js tests,
most of them `.any.js` files that need a generated page and many a
Python handler. The harness gains testharness.js: the real one,
vendored, with a `testharnessreport.js` of ours that writes the results
into the page for the runner to read; an `.any.js` test runs in a
generated `.any.html` page (window scope only); each subtest's result
is compared with `expectations.txt`, which names failing subtests. The
rule takes a testharness test when it uses no Python handler, no
`.sub` substitution and no host but its own. Directories added:
`fetch/api/headers`, `fetch/api/request`, `fetch/api/response`,
`fetch/api/basic`, `xhr`, `url` (the `urlsearchparams-*` tests) and
`FileAPI/blob`. *(corrected)* The rule also leaves out what has no
global here or needs wptserve's own timing: tests over `https:`,
workers, frames in the markup or made by a script, and pipes; a file's
`.headers` and an `.asis` file are served as wptserve serves them. The
page's clock waits for the network while a fetch is in flight. 156
tests, 1,124 of 1,469 subtests passing; an expectation names a subtest
by its path, a tab, its name, a tab and the reason.

**Golden cases** continue from 126: a page whose boxes are built from a
`fetch` of a `data:` URL, and one from an XHR's, both captured from
the reference browser on its virtual time. *(corrected)* The
reference runner waits for the network before each step of its
clock.

**Acceptance.** Beyond the adversarial page: a page served by `net`'s
test server that signs in by a `fetch` `POST` (the server answering
with a `Lax`, `HttpOnly` session cookie), shows the signed-in state
from a second request that the cookie authorizes, and loads data from
a second origin by CORS with credentials, through the shell's window.
*(corrected)* And in the real shell, the page signing in on load.
A benchmark, `benches/fetch.rs`: a thousand `fetch`es of a small JSON
body from the test server and the time to the last promise, an XHR's,
and the jar's retrieval with 3,000 cookies kept.

## 2. Not in this milestone, said rather than hidden

- *Streams*: `ReadableStream` and the rest of the Streams Standard,
  `Response.body`, a request body streamed (`duplex`), progress
  events as bytes arrive. G67.
- *The `URL` class*, its setters (the URL parser's state overrides),
  `URL.createObjectURL` and `blob:` URLs. G68, done in gap batch A
  (`docs/design/scripting-essentials.md`).
- *`TextEncoder` and `TextDecoder`*. G69, done in gap batch A.
- *XHR's `document` response type*, `responseXML`, a `Document` sent as
  a body, and `DOMParser`, which need a second document. G70.
- *HTTP authentication*: a 401 or 407 answered with credentials, the
  `Authorization` header from a URL's credentials (such URLs are refused
  by `fetch`, as the reference browser refuses them). G71.
- *The Cookie Store API* (`cookieStore`). G72.
- *`keepalive`, `navigator.sendBeacon` and `fetchLater`*: requests that
  outlive their page. G73.
- *WebSockets and server-sent events* (`EventSource`). G74.
- *Cookies on disk*, and the HTTP cache that `Request.cache` names. M28
  and M38.
- *`Referer` and Referrer Policy*, HSTS, Upgrade-Insecure-Requests and
  the mixed-form warning. M33.3.
- *Third-party cookie policy*, partitioned cookies (CHIPS), pools and
  caches partitioned by site. M33.
- *Opaque Response Blocking*, and with it the isolation of a process
  from the bodies it may not read. M34.
- *Subresource Integrity* for `fetch`'s `integrity`. G43.
- *Fetch in workers*, which have none yet. M31. Service workers, M39.
- *Frames*, and with them `document.domain`'s effect and
  `postMessage`. M33.2 and M31.
- *(corrected)* Found while working, and registered: `FileReader`
  (G75); HTML's `formdata` event (G76); a response whose header section
  ends at the connection's close, which `net` takes as truncated and
  the reference browser reads (G77).

## 3. Order of work

Each step a vertical slice, with its tests, a sabotage pass and the
gate.

1. *Origins and sites*: `Page::origin`,
   `Url::is_potentially_trustworthy`, the PSL (D20), `Origin::site`
   and `same_site`; `self.origin`, `isSecureContext`,
   `document.domain`; the existing same-origin tests on the kept
   origin.
2. *Requests and responses in `net`*: methods as tokens, author
   headers, `net::headers`' lists, modes, credentials, redirect modes,
   `Origin`, the tainted origin, `Sec-Fetch-*`, tainting and
   `Response::exposed`, `Cross-Origin-Resource-Policy`.
3. *CORS*: the check, the preflight and its cache, CORS on redirects;
   the CORS settings attribute for `script`, `link` and `img`, a
   script's errors unmuted by tainting. *(corrected)* The check came
   with step 2's tainting; `img`'s attribute waits for M30.
4. *Cookies*: `net::cookies`, the jar in `Client`, `Set-Cookie` on
   every hop, `Cookie` by credentials and same-site, navigations'
   client; `document.cookie`, `navigator.cookieEnabled`.
5. *Bodies*: `Blob`, `File`, `FormData`, `URLSearchParams`, buffers
   across the boundary.
6. *`fetch()`*: `Headers`, `Request`, `Response`, the `Body` mixin,
   `Call::Fetch`, `Target::Fetch`, the networking tasks,
   `Task::Fetched`, aborts, the limits.
7. *`XMLHttpRequest`*: the object, its events and `ProgressEvent`, the
   upload, timeouts, synchronous requests.
8. *Mixed content* for what scripts fetch, and the CORS-mode image.
   *(corrected)* And why a script's fetch failed, on the console.
9. *The adversarial page* (M27.7).
10. *Fuzzing*: the `cookies` target, `dom_api`'s new calls.
11. *The web-platform tests*: testharness.js in the harness, the
    directories, expectations.
12. *Golden cases* from 126.
13. *Acceptance*: the signed-in page through the shell, the
    benchmark, T4.3, fuzz runs.
14. *The documents*.
