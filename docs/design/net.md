# Design note: `net` v1, URLs and HTTP/1.1

Roadmap M14. Written before the code, as the record of the decisions;
what ships is described in `docs/architecture.md` "Current state" when
the code lands. Where this note and the code later disagree, the code
wins and this note is corrected, as the notes before it were.

The problem: the browser shows only what it is handed as a string. The
engine takes a document as text (`Engine::load_document`), its only
stylesheets are the text of `<style>` elements, and `net` is a stub
that re-exports the `url` crate's `Url` and whose one function returns
`NotYetImplemented`; nothing in the workspace calls it. Phase B of the
roadmap, "type a URL, see a page", starts here. This milestone builds
what loading needs beneath navigation: URLs as the URL Standard parses
them; fetching as the Fetch Standard describes it, for the `about:`,
`data:`, `file:`, `http:` and `https:` schemes; HTTP/1.1, over TLS for
`https:`; the MIME type and the character encoding of what arrives;
and the engine's loader, which fetches a document and the stylesheets
it links and puts the page together as they arrive. M15 then builds
navigation, the address bar and error pages on top.

The standards are read at the versions current when this note was
written: the URL Standard of 10 September 2026, Fetch of 21 September
2026, MIME Sniffing of 17 July 2026 and Encoding of 21 May 2026. For
documents and stylesheets it also uses HTML's encoding sniffing,
`<base>` and `<link>` sections, css-syntax-3 §3.2, css-cascade-5 §2,
css-values-4 §4.5 and CSSOM §6.2. HTTP is RFC 9110 and RFC 9112, and
the address spaces come from Local Network Access. Where a standard
leaves a choice to the user agent, the choice is stated below.

## 1. Decisions

**The URL parser is ours (D17).** `net::url` implements the URL
Standard's basic URL parser as the state machine it is written as,
with its host parser (domains; IPv4 with the IPv4 number parser's
decimal, octal and hex parts; IPv6 with compression and an embedded
IPv4 address; opaque hosts), its percent-encode sets, its serializer
and its origins. A `Url` keeps the record the standard describes —
scheme, username, password, host, port, path (opaque, or a list of
segments), query and fragment — and its serialization, made once when
it is parsed, so that `as_str` costs nothing. Parsing takes an
optional base URL and an encoding for the query. HTML parses a
document's URLs with the document's encoding, and the URL Standard
percent-encodes a query after encoding it with that encoding, writing
a character the encoding cannot represent as `&#N;`. Validation
errors are not reported: the parser returns a URL or failure, which is
all a browser acts on. The `url` crate is removed. It was weighed:
it is mature, follows the standard and is fuzzed upstream. It was not
chosen because the URL parser is web-platform semantics rather than a
non-engine concern (roadmap §1.3), because the roadmap fuzzes it as a
priority target of this project's own (T2.1, and M14.9, which names
URL parsers the browsers' most common historical source of CVEs), and
because the parser must follow the standard exactly as the standard
moves. A dependency follows it on its own schedule. Run against the
current `urltestdata.json`, the `url` crate's latest release, 2.5.8,
gives a different answer in 65 of the 896 cases. Among them are
`https://xn--/`, which the rule for ASCII domains below accepts, and
the standard's newer handling of `file:` paths and of `^` in paths.
The URL API's setters and the `application/x-www-form-urlencoded`
parser and serializer are not built now; their users, scripts and
form submission, come later (M25, M27).

**Domains through the `idna` crate.** The URL Standard maps a domain
through Unicode IDNA Compatibility Processing (UTS #46): ToASCII with
CheckHyphens, UseSTD3ASCIIRules and VerifyDnsLength off, CheckBidi
and CheckJoiners on, and nontransitional processing. That needs
Unicode's mapping, normalization and bidi data, which the `idna`
crate carries through ICU4X, the library D7 names for M42. Since the
standard's current revision, a domain that is all ASCII is
lowercased and used as it is, whatever ToASCII would say of it
(`https://xn--/` parses), and only a domain with a non-ASCII code
point goes through ToASCII. The parser makes that split itself and
calls `idna` only for the second case. It checks the standard's
forbidden domain code points itself as well.

**The Encoding Standard is a crate of its own (D18).** The HTML parser
needs encodings to find a document's, `css` to decode a stylesheet,
`net` to encode a URL's query, and the engine to decode what it
fetches. Those are siblings in the layer graph, so the type belongs
below all of them (the layering contract's rule 3): a new leaf crate,
`encoding`, beside `dom` and depending on nothing. It has:

- the standard's 40 encodings and their 228 labels;
- get an encoding, get an output encoding, BOM sniffing and decode;
- decoders written as the standard writes them: state machines fed
  bytes in pieces of any size and told when the input ends, with the
  replacement and fatal error modes.

This milestone gives it these decoders: UTF-8, UTF-16BE, UTF-16LE,
the 28 single-byte encodings, x-user-defined and replacement. It gives
it two encoders: UTF-8, and the single-byte encodings with the fatal
and html error modes, which a URL's query uses. The single-byte
tables are generated from the standard's index files, 27 files and
3,342 entries. The repository keeps the files and a test that
regenerates the tables and compares them. The standard puts the
portions of it carried into source code under BSD-3-Clause, and that
notice is kept with them.

`encoding_rs` was weighed. It is complete and fast, but it is Gecko's
encoding converter, and the roadmap's first non-goal rules out
reusing Gecko code (§1.3).

The legacy CJK encodings are GBK and gb18030, Big5, EUC-JP,
ISO-2022-JP, Shift_JIS and EUC-KR. Their index tables hold some
74,000 entries, against the single-byte encodings' 3,342. They are
recognised by label now. They are decoded once a page the T1 track
measures needs one (risk R1, no scope without a failing real page),
and at the latest with M42's internationalization. Until then a
document in one of them is decoded as UTF-8, the fallback below. That
keeps its ASCII and turns the rest into U+FFFD. The text is
unreadable, and `Engine::undecodable_encoding` names the encoding it
could not decode, which the shell prints.

**One way to fetch: `net::Client::fetch`.** A `Request` names:

- a URL;
- a method, `GET`, `HEAD` or `POST`, and a body;
- a destination, `document` or `style` for now;
- a mode, `navigate` or `no-cors`;
- an initiator: the URL and address space of the document that asked,
  or none for a load the user asked for;
- a handle that aborts it.

`Client::fetch` blocks its thread and returns a `Response` or a
`NetworkError`. A response carries:

- the URL list, every URL that redirects led through;
- the status and its message;
- the header fields and the body;
- the MIME type extracted from its `Content-Type`;
- the address space of the server it came from: none for `about:` and
  `data:`, which no server answers, and loopback for `file:`.

A `NetworkError` names the step that failed:

- the scheme or the port;
- a blocked address space, or mixed content;
- name resolution, or connecting;
- TLS, with the certificate's fault;
- a malformed response, or a limit;
- a timeout, the redirect limit, or an abort.

Aborting wakes the connection the fetch is waiting on, so a read, a
write or a TLS handshake ends at once; resolving a name, connecting
and waiting for a free connection look at the handle every quarter of
a second (*Waiting*, under Connections below). A response is
whole: its body is read to the end before `fetch` returns, "full
buffer first" as M27.1 says. Streamed bodies come with `fetch()` and
downloads (M27, M37).

**Schemes, as Fetch's scheme fetch has them.**

- `about:blank` is an empty `text/html;charset=utf-8` response. Any
  other `about:` URL is a network error; M15 adds `about:silver` as a
  navigation, not a fetch.
- `data:` URLs go through Fetch's data: URL processor:
  1. take the URL's serialization without its fragment;
  2. the MIME type is what comes before the first comma, and the rest
     is percent-decoded;
  3. after `;base64`, the rest is decoded by Infra's forgiving-base64;
  4. a type that does not parse becomes `text/plain;charset=US-ASCII`.
- `http:` and `https:` go to HTTP fetch, below.
- `file:` is left to the user agent by Fetch ("when in doubt, return a
  network error"). Here, a `file:` URL with an empty host reads the
  file its path names: the path's percent-encoded bytes are decoded,
  and on Windows a drive letter's path is used. The type comes from
  the extension for the few types the engine knows (`html`, `htm`,
  `css`, `txt`, and the image types M23 reads), and from MIME
  Sniffing's rules for an unknown type otherwise. A directory, a
  non-empty host, or a file over the body limit is a network error.
- `blob:` is a network error until there are blobs (M27).
- `javascript:` is never fetched; M15's navigation decides what it
  does.
- Every other scheme is a network error.

**Redirects, as Fetch's HTTP-redirect fetch has them.** A 301, 302,
303, 307 or 308 response with a `Location` is followed (redirect mode
`follow`, the only one this milestone uses):

- The location is read as UTF-8, as browsers read it (Fetch does not
  say), and parsed against the URL that answered. Several `Location`
  fields, or one that does not parse, are a network error.
- A location without a fragment takes the request's fragment.
- A location that is not `http:` or `https:` is a network error, and
  so is the twenty-first redirect.
- A 301 or 302 answering a `POST`, or a 303 answering anything but
  `GET` or `HEAD`, turns the request into a `GET` with no body. Fetch
  drops the request's `Content-Type`, `Content-Encoding`,
  `Content-Language` and `Content-Location` with it; a request here
  has no header list of its own yet and sends none of them, so there
  is nothing to drop.
- Fetch drops `Authorization` on a redirect to another origin. Nothing
  sends it yet; the rule comes with the request's header list (M27).
- A 300 or 304 with a `Location` is an answer, not a redirect: Fetch's
  redirect statuses are the five above.

Each hop runs again every check a fetch starts with: scheme, port,
address space and mixed content. A redirect cannot reach what the
first URL could not.

**HTTP/1.1 is ours too (D5).** The register's working answer, "custom
lite first", stands. `net::http` writes requests and reads responses
over any byte stream, plain or TLS, so the same code is tested over an
in-memory stream and fuzzed.

A request has an origin-form target (the path and query, never the
fragment), `Host` with the port when it is not the scheme's default,
the fields below, and `Content-Length` when there is a body. It is
never pipelined.

A response is read per RFC 9112:

- The status line must say `HTTP/1.1` or `HTTP/1.0`; anything else is
  an error.
- A line may end in LF alone (§2.2).
- A field name must be a token. Obsolete line folding is replaced by
  spaces, as §5.2 asks of a user agent. A field value holding NUL, or
  a CR that is not part of a line ending, is an error. A line that
  begins with white space before any field it could continue is
  ignored, one of the two answers §2.2 allows.
- 1xx interim responses are read and skipped. A 101 is an error: there
  is no upgrade to accept.

The body's length follows §6.3, in this order:

1. No body for a `HEAD` request or a 1xx, 204 or 304 response.
2. If the last coding in `Transfer-Encoding` is `chunked`, the body is
   chunked. Chunk extensions and trailer fields are read and dropped.
3. Any other `Transfer-Encoding` means the body is read until the
   connection closes. So does any `Transfer-Encoding` in an HTTP/1.0
   response, whose framing §6.1 calls faulty.
4. Otherwise `Content-Length` gives the length. Several values must
   agree, and an invalid one is an error.
5. Otherwise the body is read until the connection closes.

A transfer coding overrides `Content-Length`. A message with both may
be an attempt at response splitting, so its connection is not reused.
A body shorter than its declared length is an error, never a page cut
short. Bytes that arrive after a response, before the next request,
mean the connection is not reused either: they would be read as the
next response.

`hyper`, the register's alternative, would bring an asynchronous
runtime (below) and its own connection management. The parts of
HTTP/1.1 a browser client needs are small enough to own and to fuzz.

**Connections.**

- *Names.* The operating system's resolver (`ToSocketAddrs`, that is,
  getaddrinfo) resolves a host. The exceptions are `localhost` and
  names ending in `.localhost`, which resolve to `::1` and `127.0.0.1`
  without asking anyone, as Fetch's resolve an origin says. An IP
  address is used as it is. The resolver runs on a thread of its own,
  which an abort leaves to finish by itself.
- *Connecting.* The addresses are tried in the resolver's order,
  alternating IPv6 and IPv4. If none has connected yet, a new attempt
  starts 250 milliseconds after the last one began (RFC 8305's
  connection attempt delay). The first connection that succeeds is
  used, the rest are closed, and all the attempts together get 30
  seconds.
- *Pooling.* Connections are pooled by origin: scheme, host and port.
  One is reused after an HTTP/1.1 response that did not say
  `Connection: close` and whose body was read to its declared end. At
  most six are open to one origin (M14.3); a seventh request waits for
  one to be free. An idle connection is closed after 30 seconds, and
  is looked at before it is taken: one the server closed, or sent
  anything on (a 408, say), gives its place up. Without that, a `POST`
  sent after a server's shorter keep-alive, Apache's five seconds,
  would fail, since a `POST` is not retried. A pooled connection's
  address is checked against the request's initiator as a new one's
  is.
- *Retry.* A `GET` or `HEAD` sent on a reused connection that fails
  before any byte of a response arrives is sent once more, on a new
  connection (RFC 9110 §9.2.2, RFC 9112 §9.3.1). The server may have
  closed the idle connection just as it was taken.
- *Waiting.* Sockets are non-blocking. Each connection has a poll of
  its own, registered for reading and writing, with a waker: `mio`,
  the operating systems' readiness APIs and nothing more, used on the
  fetch's own thread (MIT; its dependencies were in the tree already).
  A read, a write or a TLS handshake tries, and when the socket would
  block waits in the poll for what is left of its time (below). A
  fetch's abort handle keeps the wakers of the connections it is using
  and wakes them, so an abort ends the wait at once on every OS. No
  socket timeouts are set; a wait that runs out ends that connection's
  use. Resolving, connecting and waiting for a free connection look at
  the handle every 250 milliseconds instead.
- *Not yet.* The pool is not partitioned by top-level site (Fetch's
  network partition key). That is privacy work for M33, together with
  the TLS session cache below. There is no proxy support, and no
  roadmap item has it yet.

**TLS: rustls over aws-lc-rs, with Mozilla's roots (D4).** `https:`
connects through `rustls` with:

- TLS 1.2 and 1.3 only;
- the host name sent by SNI, except for an IP address;
- ALPN offering `http/1.1` alone;
- 0-RTT off;
- session resumption on, kept in memory.

The cryptography comes from rustls's `aws-lc-rs` provider rather than
`ring`, its other one. Only aws-lc-rs has the post-quantum hybrid key
exchange X25519MLKEM768, which rustls offers first by default and the
major browsers ship. With it, a quantum computer built later cannot
recover the keys of traffic recorded now. The price was measured on a
probe that only builds a TLS client, in release:

| Provider | Binary | Build time |
|---|---|---|
| `ring` | 2.2MB | 19s |
| `aws-lc-rs` | 4.4MB | 40s |

Both licence sets are on the allow list: ISC, Apache-2.0, MIT and
BSD-3-Clause. No OpenSSL licence remains in aws-lc's.

Certificates are checked by rustls's WebPKI verifier against
`webpki-roots`, Mozilla's root program compiled in, rather than the
operating system's store. Every OS gets the same roots, as browsers
that run their own root programs do, and none depends on what an OS
update or another program installed.

A certificate that fails is a network error saying why: expired, not
yet valid, unknown issuer, or the wrong name. There is no way to
accept it anyway (M14.4: fail closed). Tests trust a root of their
own (below); the shell has no option to add one.

Not checked in v1, and all left to M33's sweep:

- revocation (no OCSP, no CRLs, no CRLite-style set);
- Certificate Transparency;
- Encrypted Client Hello, which needs DNS HTTPS records.

**Content codings.** Requests send `Accept-Encoding: gzip, deflate`.
Those codings are decoded with `flate2` over `miniz_oxide`, pure Rust
and already in the tree. `deflate` is read either zlib-wrapped, as RFC
9110 defines it, or raw, as some servers send it and browsers accept.
A body that fails to decode is a network error. Any other coding
passes through undecoded, as Fetch's handle content codings says. The
decoded size counts against the body limit, so a small compressed
body cannot grow past it. `br` and `zstd` are not asked for: they save
bytes over gzip but are not needed to read a page. M35's performance
work measures them.

**Limits and time.**

- A response's status line and header fields, 1xx responses included,
  may total 256 KiB.
- Its body, decoded, may be 64 MiB. A chunked body's size lines and
  line endings may take as many bytes again, and its trailer fields
  count against the header's limit.
- A read that waits 60 seconds for a byte, a write that waits as long
  for room, or a TLS handshake that makes no progress for as long ends
  the fetch with a timeout.

These are constants in `net`, each with its reason beside it. Name
resolution runs on the resolver's own time limit, and an abort cannot
interrupt it, since std's resolver blocks: the fetch is abandoned and
the thread finishes on its own.

**Threads, not an asynchronous runtime.** `Client::fetch` blocks. The
engine calls it from threads of its own, at most 32 at once: a thread
takes the next waiting fetch when it finishes one and ends when none
waits, and further fetches wait in the order they were made. A browser
page has tens of requests in flight, not thousands. A thread each
costs its stack, committed only as it is used, and nothing in
dependencies. An
asynchronous runtime (tokio, under hyper) would be the largest
dependency in the tree and would make every layer above it choose an
executor. The register's D5 records this with the HTTP decision.

**MIME types and sniffing (M14.5).** `net::mime` parses and serializes
MIME types (MIME Sniffing §4.4 and §4.5). It extracts a response's
type as Fetch §3.5 does: the last valid `Content-Type`, with a charset
carried over while the essence stays the same. A document's computed
type comes from the MIME type sniffing algorithm (§7):

- A supplied HTML or XML type is trusted.
- A missing type, or `unknown/unknown`, `application/unknown` or
  `*/*`, is identified from the first 1445 bytes by §7.1's rules,
  scriptable types included unless `X-Content-Type-Options: nosniff`
  says otherwise.
- The four exact `text/plain` values old Apache servers sent for
  everything go through §7.2's text-or-binary rules.
- An image or media type the engine supports is checked against its
  signature. There are none until M23.

A stylesheet's type is what HTML's stylesheet processing takes:

- A response with no `Content-Type`, or one that does not parse, is
  taken as `text/css`, unless it says `X-Content-Type-Options:
  nosniff`.
- Any other type than `text/css` means the sheet is not applied.

HTML's exception for documents in quirks mode does not arise: this
engine has none (D14). With `nosniff`, Fetch blocks a style response
whose type is not `text/css` or is missing: the first is the rule
above already, and the second is why a missing type is not taken as
`text/css` then.

**Decoding a document.** `html` takes bytes now, with the transport
layer's `charset`. It runs HTML's encoding sniffing algorithm and
returns the document, the encoding, and its confidence:

1. the BOM;
2. the `charset` from the transport layer;
3. a prescan of the first 1024 bytes for `<meta charset>` or `<meta
   http-equiv=content-type>`;
4. the fallback.

The fallback is UTF-8, as M14.5 has it, not the locale's default that
HTML suggests (windows-1252 for most locales). HTML allows either: it
suggests UTF-8 where encodings can be prescribed and leaves the choice
to the user agent. A page that declares nothing and is not UTF-8 shows
its non-ASCII bytes as U+FFFD: that is the roadmap's trade for not
guessing.

The whole body is in hand. So when the tree builder meets a `<meta>`
naming another encoding while the confidence is tentative, the bytes
are decoded again with it and parsed again from the start. That is
what HTML's changing the encoding while parsing comes to here: it
asks for the bytes to be parsed again from memory. The parser itself
is unchanged and still parses a whole string. HTML's encoding-parse a
URL then uses the document's encoding for every URL the document
names.

**Decoding a stylesheet.** `css` takes bytes too and finds their
encoding by css-syntax-3's determine the fallback encoding, after a
BOM, which wins over everything:

1. the `Content-Type` charset;
2. an `@charset "…";` at the very start, within the first 1024 bytes,
   with a UTF-16 label read as UTF-8;
3. the environment encoding: the link's `charset` attribute or else
   the document's encoding for a `<link>`, the importing sheet's for
   an `@import`;
4. UTF-8.

URLs inside CSS are parsed as UTF-8 (css-values-4 §4.5.4).

**What a request sends.**

- `Host`.
- `User-Agent: Mozilla/5.0 (compatible; SilverBrowser/<version>)`. It
  begins as Fetch encourages and names nothing but the browser; D12
  settles the policy at M25.
- `Accept` by destination, as Fetch lists it: a document's
  `text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8`,
  a stylesheet's `text/css,*/*;q=0.1`.
- `Accept-Language: en-US,en;q=0.9`, until settings choose (M37.3).
- `Accept-Encoding: gzip, deflate`.

Not sent:

- `Referer`. Referrer Policy arrives at M33.3; until then no request
  carries a referrer. It is the most private choice, and Fetch leaves
  it open to the user agent.
- `Cookie` (M27.4).
- `Origin`, which goes with CORS and with methods other than `GET`
  and `HEAD` (M27). Nothing in this milestone sends another method.
- The `Sec-Fetch-*` metadata fields. `Sec-Fetch-Site` needs the
  Public Suffix List, which arrives with cookies (M27).

*(corrected at M33: each is sent now: `Cookie`, `Origin` and the
`Sec-Fetch-*` fields since M27 (`docs/design/fetch.md`), `Referer` by
the referrer policy since M33.3.)*

**Address spaces (D19, M14.7).** Every address a connection would be
made to is classified by Local Network Access's table:

| Space | Addresses |
|---|---|
| loopback | `127.0.0.0/8`, `::1`, `0.0.0.0/32`, `::`, `198.18.0.0/15` |
| local | `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `100.64.0.0/10`, `169.254.0.0/16`, `0.0.0.0/8`, `fc00::/7`, `fe80::/10`, `fec0::/10`, `2001:db8::/32`, `3fff::/20` |
| public | every other address |

An IPv4-mapped IPv6 address is classified by its IPv4 address.

A request is refused before the connection is attempted when its
initiator is in a more public space than the address: a public page
asking for a local or loopback address, or a local page asking for a
loopback one. The check is made on the address about to be connected
to, so DNS rebinding cannot slip past an answer checked earlier.

- A load the user asked for has no initiator and can go anywhere.
- A page's own space is that of the server its document came from.
- A `file:` document's space is loopback.
- A document no server answered, an `about:` or `data:` one, is
  public.

M33.4 planned this posture, "block outright", for the security sweep.
It starts here because the risk is real without scripts or cookies: a
page's `<link rel=stylesheet href="http://192.168.1.1/...">` sends a
GET to a router, and devices that trust their LAN act on GETs. What
it costs is the pages that load from local servers on purpose while
served from public ones. The permission prompt Local Network Access
defines is for M33 to bring. The register records the posture as D19.

**Mixed content.** An `https:` document's subresource is blocked when
its URL is `http:` and its host is neither a loopback address nor
`localhost`; Secure Contexts counts those two as potentially
trustworthy. Stylesheets are blockable content in Mixed Content's
terms. Upgrading optionally-blockable images comes with images (M23),
and the rest of M27.5 with scripts. Like the other checks, it runs on
every redirect hop.

**Ports and schemes.** Fetch's 83 bad ports, from 0 and 1 to 10080,
are refused for `http:` and `https:` before connecting, on every hop.
A page's subresource may be `http:`, `https:` or `data:`, and `file:`
when the page itself is a `file:` page. Anything else is refused.

**The engine's loader (M14.6).** `Engine` gains a network client,
made at its first fetch or given with `Engine::set_network` (a test's
client trusts its server's root), and a waker: a function the shell
gives it with `Engine::set_waker`, which wakes the event loop from
another thread.

`Engine::load(url)` starts a load, aborting one under way. The
document is fetched on a fetch thread, and the page shown stays the
old one until the new one is ready. Each finished fetch sends its
result to the engine over a channel and calls the waker. The shell
then calls `Engine::poll_network`, which takes every finished result,
applies it, and says whether a frame is wanted. `Engine::load_state`
reports the latest load, for the shell and for M15:

- Loading, while its document, or a stylesheet the document asked
  for, is on its way.
- Loaded, once the document is shown and every stylesheet it asked
  for has arrived or failed, as HTML's `load` event waits for them. A
  sheet added later does not take it back to loading.
- Failed, with the error. The page shown is the one before.

A document given to the engine as source or as a tree
(`load_document`, `set_document`) has no URL and fetches nothing: its
`<link>` elements and `@import` rules bring no sheets, which keeps the
tests and the fuzz targets off the network. Giving one aborts a load
under way. `Engine::url` is the URL of the document shown.

When the document's response arrives:

- If its computed type is not HTML, the load fails as unsupported.
  What to show for text, images and downloads is navigation's
  decision (M15.1).
- A response with an error status, a 404 or a 500, is still a
  document, shown as the server sent it, as browsers do. M15's error
  pages are for network errors.
- Otherwise the body is decoded and parsed into a new `Document`, on
  the engine's thread. Its URL is the response's last URL, and its
  address space is the response's, public when no server answered.
- An element's URL resolves against the base URL as it stood where
  the parser inserted the element: the frozen base URL of the first
  `<base href>` before it in tree order, or else the document's URL.
  That is what a parser inserting elements one at a time gives, and
  this one parses whole (§2). A `<base>` whose URL fails to parse, or
  is `data:` or `javascript:`, leaves the document's URL in place.

The document's stylesheets then come from its `<link>` elements. A
`<link>` is fetched as a stylesheet (destination `style`, mode
`no-cors`) when:

- its `rel` has the token `stylesheet` and not `alternate`, in any
  case;
- it has no `disabled` attribute;
- its `type`, if present and not empty, is a MIME type whose essence
  is `text/css`;
- its `href` is not empty and parses against the base URL, with the
  document's encoding;
- its `media`, if present, is an empty list, or one with a query that
  is `all` or `screen`, `only` before it or not. The cascade skips
  `@media` rules until M20 evaluates them, and a sheet for another
  medium, or behind a condition, is skipped the same way. The same
  rule now applies to `<style media>`, which was applied regardless,
  and so does HTML's rule for `<style type>`: absent, empty or
  `text/css` in any case.

A stylesheet that arrives with an ok status and the right type is
decoded and parsed. Then:

1. The `@import` rules at its start are fetched the same way. They
   come after `@charset`, and after `@layer` statements before the
   first of them; any other rule ends them (css-cascade-5 §6.1). One
   with a block or no URL is invalid and passed over. One into a
   cascade layer or under a `supports()` condition is left out, as the
   cascade leaves out `@layer` and `@supports` blocks, and so is one
   whose media list does not hold. Each is resolved against the
   sheet's location: the URL it was requested from, the first of its
   URL list (css-values-4 §4.5.4, CSSOM §6.1). Imports go at most 16
   deep, a chain of 17 sheets, and an import of a sheet already in its
   own chain, by its URL without the fragment, is skipped. The
   imported rules take the place of the `@import` in cascade order
   (css-cascade-5 §6.1). A `<style>` element's `@import` rules are
   fetched the same way, against the base URL.
2. The sheet takes its place among the document's stylesheets in the
   tree order of the elements that own them (CSSOM §6.2), not in the
   order they arrived.
3. The first titled sheet that has arrived, in tree order, names the
   preferred set, and a sheet with another title is left out, as
   CSSOM's add steps say. CSSOM names the set once, from the first
   titled sheet added, which is arrival order; tree order gives the
   same result however the sheets arrive, and the set is named again
   as they arrive and as the document changes.
4. The engine restyles. A change of stylesheets cascades the whole
   document, as it did before.

A stylesheet that fails is left out, and the page renders without it.
It fails on a network error, a status that is not ok, or the wrong
type.

*Render-blocking.* HTML makes a stylesheet `<link>` or `<style>` that
the parser creates while the document has no body render-blocking:
here, one before the `html` element's `body` child in tree order. The
new document is not shown until each such sheet, with its imports, has
arrived or failed. Until then the old page stays: the page first
appears styled, never unstyled. HTML allows a timeout here and leaves
its length to the user agent. The timeout is the fetches' own: a sheet
blocks until it arrives, fails, or waits 60 seconds for a byte.

A stylesheet that turns up later does not hold back frames; the page
restyles when it arrives. The engine notices changes through the
journal. A subtree inserted or removed that holds a `<link>` or a
`<style>`, a change to one of their attributes, or a change to a
`<style>`'s text has the document's sheets found again:

- an element whose sheet was made from the same attributes (a link's
  `href`, `rel`, `type`, `media` and `disabled`, a style's text,
  `type` and `media`) keeps its sheet, even when it moved;
- one whose attributes or text changed is made or fetched again,
  against the base URL as it now stands;
- one removed takes its sheet with it, and its fetches are aborted;
- titles are read again, and the preferred set named again.

A `<base>` alone changes no sheet: HTML's base URL change steps leave
`<link>` elements alone, and the next sheet made reads the base URL as
it then stands.

A new load aborts the one under way, a new page aborts the fetches of
the page it replaces, and an engine dropped aborts them all.

**The shell.** It runs as `silver-browser [URL or path]`:

- An argument naming an existing file is loaded as that file's
  `file:` URL (`Url::from_file_path`).
- Any other argument must parse as an absolute URL, and is loaded.
  The file comes first because on Windows `C:\page.html` parses as a
  URL too, of the scheme `c`.
- With no argument, the built-in page shows, as now. More than one
  argument, or one that is neither, is a usage error.

While the first load runs, the window shows an empty page. A load that
fails is reported on standard error and the built-in page shows
instead; a page in an encoding the engine cannot decode yet is
reported too. Error pages are M15's. The event loop takes a user event
for the waker.

**Tests (M14.8).** All of them live in the repository, and none needs
the network:

- *URLs:* the URL parser against the web-platform-tests files
  `urltestdata.json` (896 cases), `toascii.json` (87) and
  `IdnaTestV2.json` (2,670; hosts through the URL parser, the empty
  input skipped as web-platform-tests' own script skips it), and
  `percent-encoding.json` (7, run nine times in the encodings this
  milestone has, the five CJK ones skipped by name). That is 3,660
  cases.
  `setters_tests.json` waits for the setters.
- *Encodings:* every label in `encodings.json`. Every byte of every
  single-byte encoding, decoded and encoded again against its index.
  The UTF-8 decoder on the standard's error cases. Decoding in random
  pieces gives what decoding all at once gives.
- *MIME:* web-platform-tests' `mime-types.json` (74) and
  `generated-mime-types.json` (881), each parsed and serialized, and
  those that can be a field value extracted as a response's
  `Content-Type` too (65 and 862). Two of those, where Fetch splits a
  value at a comma and the file does not, are held to Fetch. The
  files' minimized types are not checked: minimizing has no user yet.
  `data:` URLs against
  `data-urls.json` (72) and `base64.json` (80). Sniffing case by case
  for each rule.
- *HTTP:* the message layer over in-memory streams: each length rule,
  chunked edge cases, folding, the limits, short bodies and 1xx.
- *A test server* in `net`, behind a `test-server` feature that `net`'s
  and the engine's tests use. It is a thread on 127.0.0.1 answering
  scripted bytes per connection, plain or over TLS. The TLS root is
  generated when the tests run (`rcgen`, a dev-dependency), so nothing
  expires. The server counts connections. Against it the tests check
  reuse, the six-connection limit, the retry after an idle close,
  redirects and their method rules, timeouts, aborts, gzip, and every
  address-space and mixed-content refusal. An initiator's address
  space is set by the test, since a test cannot be served from a
  public address.
- *The engine:* a page whose stylesheets come from the test server
  draws the same frame as the same page with the sheets inline. The
  tests also cover render-blocking, `@import`, `<base>`, tree order
  against arrival order, the preferred set, types and failures,
  encodings, the journal's changes, aborts, a page no server answered,
  and the thread cap.
- *A live test:* one test fetches `https://example.com/` for real,
  against the real roots. It is `#[ignore]`d and run by hand.

The web-platform-tests files keep their BSD-3-Clause notice beside
them.

**Fuzzing (M14.9).** Four new targets:

- `url_parse`, T2.1's name: any input, alone and against a base. It
  never panics, and a URL's serialization parses back to the same URL.
- `encoding_decode`: every decoder over any bytes. It never panics,
  and decoding in pieces gives what decoding at once gives.
- `mime_sniff`: parsing a MIME type and serializing it parses back the
  same, and sniffing never panics.
- `http_response`: the response reader over any bytes, cut into any
  pieces. It never panics and never reads past its limits.

CI runs them with the others.

**The layering contract.**

- `encoding` joins `dom` at the bottom of the graph.
- Rule 2's "only `shell` touches the OS" is about windows and input.
  It gains a sentence: only `net` opens network connections and
  reads resources by URL. CI enforces it by T3.4's grep: `TcpStream`,
  `UdpSocket`, `ToSocketAddrs` or `rustls` in a `.rs` file or a
  `Cargo.toml` under `crates/` or `fuzz/`, outside `crates/net`, fails
  the build.
- Rule 6 lets `net` use `#[cfg(windows)]` for one thing: turning a
  `file:` URL's path into a drive-letter path.

## 2. Not in this milestone, said rather than hidden

- *Parsing as bytes arrive.* HTML specifies it from the start: bytes
  are fed to the parser as they come. Here the document is parsed once
  its body is complete, and then built from its resources as they
  arrive; that is M14.6's incremental assembly as this milestone reads
  it. Parsing as bytes arrive needs two things. The tokenizer must be
  rewritten as the specification's state machine, resumable at any
  byte; today it is a pragmatic subset that reads a whole string. The
  tree builder must write into the `dom::Document` as it goes. Both are
  bound up with what scripts do to the input stream: the insertion
  point, `document.write` and a parser paused by a script. So the work
  becomes a new item with scripts, M24.8, rather than being designed
  twice. The M13 documents that promised it to M14.6 are corrected.
- *Images* are fetched with M23, which decodes them.
- *Cookies, CORS* and the same-origin policy are M27, and so is
  Cross-Origin-Resource-Policy on no-cors responses. *Referer* and
  *HSTS* are M33.3. The *cache* is M28.4 and M38.2, *HTTP/2* is M38,
  and the *preload scanner* is M38.3.
- *Revocation, Certificate Transparency, ECH,* and *pools and session
  caches partitioned by site* are M33.
- *Proxies*: not on the roadmap yet.
- The *legacy CJK encodings* come when a T1 page needs them, and at
  the latest with M42.
- The URL API's *setters* and *`application/x-www-form-urlencoded`*
  come with their users, scripts and forms (M25, M27).
- *Streamed bodies and downloads* are M27.1 and M37.2. *`br` and
  `zstd`* go to M35 to measure.
- From M15: *navigation, error pages, `about:silver`, and documents
  that are text or images*.
- *Stylesheets from an HTTP `Link` field*: HTML says such a field does
  nothing for `stylesheet`.
- *`crossorigin` and `integrity` on `<link>`* come with CORS (M27).
- *Alternate stylesheet sets* the user switches between: they need a
  UI.
- *Quirks mode's stylesheet type exception*: there is no quirks mode
  (D14).
- *Media queries* beyond `all` and `screen` are M20's. A sheet behind
  one is left out until then, and so is an `@import` into a cascade
  layer or under `supports()`, until the cascade has layers and
  `@supports`.

## 3. Order of work

Each step is a vertical slice: build, tests, clippy green and a smoke
run before the next. A step is done when its tests pass and its fuzz
target runs clean.

1. *`encoding`.* Labels, the decoders and encoders above, the tables
   generated from the index files, and the `encoding_decode` fuzz
   target.
2. *The URL parser.* Hosts through `idna`, the serializer and origins.
   The URL test files and the `url_parse` target. The `url` crate goes.
3. *MIME types.* Parsing, serializing, extracting and sniffing,
   `data:` URLs with forgiving-base64, their test files, and the
   `mime_sniff` target.
4. *Documents and stylesheets from bytes.* HTML's encoding sniffing,
   with the prescan and the re-parse on a late `<meta>`. css-syntax's
   decode.
5. *The HTTP message layer* over any stream, with the
   `http_response` target.
6. *Connections.* Resolution, connection attempts, address spaces,
   TLS with its roots, the pool, retry, content codings, limits,
   timeouts and aborts. The test server and its TLS root.
7. *Fetch.* Schemes, redirects, bad ports, the address-space and
   mixed-content checks, the request's fields, and `Client::fetch`.
   The ignored live test.
8. *The engine's loader and the shell.* `load`, `poll_network`, the
   waker, documents committed when their render-blocking sheets
   settle, stylesheets in tree order with their imports, `<base>`,
   media and the preferred set, and journal-driven updates. The
   shell's argument. T3.4's grep in CI.
9. *The documents.* This note corrected to the code, the
   architecture's M14 entry, the roadmap's ticks, and the numbers for
   `docs/perf-budgets.md`: the time from `load` to the first frame for
   a page of 1,000 paragraphs and three stylesheets from the local test
   server.

All nine landed on 2026-10-02. Step 1 took a second commit, for the
tables' line endings on a Windows checkout; step 6 a second, the waker
that replaced shutting a socket down to abort it; and step 8 two, the
loader and the shell. Step 9 also gave the engine the report of an
encoding it cannot decode, which this note had promised and step 8
missed.

## 4. What the code taught

Six things this note did not foresee, each corrected above where it
stands. The note had an abort noticed between socket reads of a
quarter of a second; step 6 shut the socket down from the aborting
thread instead, and on Windows that does not end a call already
waiting on it: CI ran four abort tests there for their full minute.
Sockets became non-blocking, each waiting in a poll that an abort
wakes. A pooled connection the server had closed while it sat idle was
taken, and a `POST` on it failed, since a `POST` is not retried, so an
idle connection is now looked at before it is taken. A `<link>` before
a `<base>` resolves against the document's URL, since a parser inserts
the `<link>` while no `<base>` exists; the note had one base URL for
the whole document. With `nosniff`, Fetch blocks a style response with
no type at all, which the note took as `text/css`. An `@import` into a
layer or under `supports()` has no cascade to go to yet, and the rule
for where imports may stand is css-cascade-5's, `@layer` statements
and all. And watching `<base>` changes, which the note planned,
changes nothing a page can see: breaking that code on purpose failed
no test, because a base URL change refetches no sheet and every later
sheet reads the base URL afresh, so it went.
