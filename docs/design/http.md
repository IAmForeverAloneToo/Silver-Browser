# Design note: HTTP/2, the cache, and the preload scanner

Roadmap M38. Written before the code, as the record of the decisions,
and corrected to the code when the milestone ends: where they
disagree, the code wins, and section 4 lists the places, marked
*(corrected)*. What ships is described in `docs/architecture.md`
"Current state", and the limits it adds live in `docs/security.md`.

The browser speaks HTTP/1.1 alone (M14, `docs/design/net.md`). A fetch
runs on a thread of its own and blocks: it takes a connection from the
pool (six an origin, keyed by scheme, host, port and partition), writes
its request, reads the response and gives the connection back. One
request is on a connection at a time. TLS offers ALPN `http/1.1` and
nothing reads what was chosen, and the response reader refuses an
`HTTP/2` status line. Each tab makes its own client, so two tabs of a
profile do not share connections. The HTTP cache (M28.4,
`docs/design/storage.md`) is RFC 9111's private cache in memory: by
partition and URL with `Vary`'s variants, fresh by `max-age`,
`Expires`, `Age`, `Date` and the 10% heuristic, read and written by
Fetch's cache modes, within 64 MiB. It keeps no validator and sends no
conditional request, so a stale entry is fetched whole again; it
honours `no-store`, `no-cache` (as never fresh), `max-age`, `public`
and `private` and nothing else; documents never pass through it (a
streamed fetch neither reads nor writes it); a reload is an ordinary
navigation; and it is gone when the browser ends. The parser fetches
what it meets as it meets it: a stylesheet when its `<link>` is
inserted, a script when its `</script>` is reached, an image only once
the document is shown. While a parser-blocking script waits for its
bytes, nothing after it is asked for.

This milestone gives the network what a page of today's web expects:
HTTP/2 over TLS, many requests on one connection; a cache that
revalidates, keeps documents, and is kept in the profile across runs;
and a scanner that reads ahead of the parser and asks for what the
document will need. Its acceptance (M38.4): a page of twenty resources
loaded a second time makes no network request, and HTTP/2's
multiplexing is seen at a test server.

The documents this note follows, at the versions current when it was
written (21 October 2026):

- RFC 9113 (HTTP/2), RFC 7541 (HPACK), RFC 9218 (Extensible
  Prioritization), RFC 8740 (HTTP/2 with TLS 1.3), RFC 7301 (ALPN);
- RFC 9111 (HTTP Caching), RFC 9110 (HTTP Semantics: validators and
  conditional requests), RFC 8246 (`immutable`);
- the Fetch Standard: "HTTP-network-or-cache fetch" (its cache modes,
  revalidation, the 304), and the fetch params of navigations;
- the HTML Standard: the speculative HTML parser (13.2.7), `<link
  rel=preload>` (the preload cache and its matching), reload and
  history traversal's cache behaviour.

## 1. Decisions

### HTTP/2 (M38.1)

**Our own, in `net::h2`.** Framing (RFC 9113 section 4 and 6) and HPACK
(RFC 7541: the static table, the dynamic table, Huffman coding both
ways) are written in the crate, as HTTP/1.1 was: no new dependency, and
every byte read from a server goes through code the crate's tests and
fuzz targets hold. `net::h2::hpack` and `net::h2::frame` are pure
(bytes in, values out); `net::h2::conn` is the client's connection.

**Over TLS alone, chosen by ALPN.** TLS offers `h2` then `http/1.1`;
the connection speaks what the server chose (`alpn_protocol()`), and a
server that chooses nothing speaks HTTP/1.1. Plain `http:` stays
HTTP/1.1: no `h2c`, no prior knowledge, as browsers do. rustls's TLS
1.2 suites are all ephemeral with AEAD, which RFC 9113 9.2.2 asks.

**A connection thread, many fetch threads.** A fetch still runs on its
own thread and blocks; an HTTP/2 connection is owned by a thread of its
own that reads and writes its socket (the same mio wait and abort
machinery as today) and routes each frame to its stream. A fetch opens
a stream by handing the connection its request through a queue and a
waker, and reads its stream's events from a channel of its own: the
head, pieces of the body, the end, or an error. Above the transport
nothing changes: `exchange` and `exchange_streaming` give back the same
`Response`, its version `Http2`, its body decoded as today.

**The pool knows both.** `OriginKey` keeps its fields. For a key with
an HTTP/2 connection that will take another stream (fewer than the
server's `MAX_CONCURRENT_STREAMS`, and no `GOAWAY`), every fetch shares
it. While the first TLS handshake to a key is under way, the fetches
behind it wait for its ALPN rather than each opening a connection of
its own (the reference browser's rule); if it chose HTTP/1.1, they go
on as today, six at a time.

**The pool belongs to the context.** `engine::Context` (M37) gains the
transport, so every tab and window of a profile shares connections, the
partition in the key still keeping sites apart. A private window's
context has its own.

**The settings we send:** `ENABLE_PUSH` 0 (no server push: a
`PUSH_PROMISE` is a connection error), `MAX_CONCURRENT_STREAMS` 100
(the server's pushes alone, so moot), `INITIAL_WINDOW_SIZE`
`H2_STREAM_WINDOW` (6 MiB), `MAX_HEADER_LIST_SIZE` the HTTP/1.1
`HEADER_LIMIT` (256 KiB), `SETTINGS_NO_RFC7540_PRIORITIES` 1, and a
`WINDOW_UPDATE` raising the connection's window to
`H2_CONNECTION_WINDOW` (15 MiB), the reference browser's figures.

**Flow control as the reader reads.** The connection gives a stream's
window back (a `WINDOW_UPDATE`) as the fetch thread takes its bytes,
not as they arrive, so a slow reader holds back its own stream and no
more; the connection's window is given back as bytes are taken off the
socket. A request's body (a `POST`'s) is sent within the server's
windows.

**Headers.** The request's pseudo-headers (`:method`, `:scheme`,
`:authority` for `Host`, `:path`), its fields lowercased, the
connection-specific ones dropped (`Connection`, `Keep-Alive`,
`Proxy-Connection`, `Transfer-Encoding`, `Upgrade`; `TE` but
`trailers`). A response whose head breaks RFC 9113 8.2 and 8.3 (an
uppercase name, a pseudo-header after a field or unknown, a missing
`:status`, a connection-specific field, a `content-length` its `DATA`
contradicts) is malformed: the stream is reset with `PROTOCOL_ERROR`
and the fetch fails as `NetworkError::Malformed` does today.

**Priorities by RFC 9218.** RFC 9113 deprecates the old tree; a request
carries the `priority` field from its destination: a document and a
stylesheet `u=0`, a script `u=1`, a fetch `u=3` (the default), an image
`u=5, i`, a speculative request one urgency lower than its kind. No
`PRIORITY` or `PRIORITY_UPDATE` frame is sent.

**Errors and retries.** A `GOAWAY` stops new streams on the connection;
the streams above its last stream id were not processed and are retried
on a new connection, as is a stream refused (`REFUSED_STREAM`), once. A
reset stream, or a connection error, fails its fetches with a new
`NetworkError::Http2(code)`. A connection idle for `IDLE_TIMEOUT`
(30 s) is closed with a `GOAWAY`. A `PING` is answered; an abort resets
its stream with `CANCEL`.

**Connection coalescing (RFC 9113 9.1.1).** A fetch to a host with no
connection of its own may use an HTTP/2 connection to another host of
the same scheme, port and partition when the host resolves to the
connection's peer address and the connection's certificate is valid for
the host (`rustls::client::verify_server_name`), and its address space
is the same. A `421 Misdirected Request` is retried on a connection of
the host's own.

**Defences.** The peer's header list is bounded by its decoded size
(`MAX_HEADER_LIST_SIZE`) and its compressed size across `CONTINUATION`
frames (twice that); the HPACK dynamic table by the size we announced,
whatever the peer's updates ask; a frame by `MAX_FRAME_SIZE` (16 KiB,
the default we keep); a stream's and the connection's buffered body by
their windows; control frames (`SETTINGS`, `PING`, empty `DATA`,
`WINDOW_UPDATE` that open nothing) by `MAX_H2_CONTROL_FRAMES` a second,
past which the connection ends with `ENHANCE_YOUR_CALM`.

**The test server speaks it.** `net::test_server` gains HTTP/2 over
TLS (ALPN `h2` when asked for), each stream's reply on a thread of its
own so that slow replies are seen to overlap, and it records each
request's connection and stream, so that tests and the acceptance see
the multiplexing.

**The network log says the protocol.** `Observed::Response` gains the
protocol (`http/1.1` or `h2`), which the DevTools network panel shows.

### The cache (M38.2)

**Validation.** A stored response keeps its validators (`ETag`,
`Last-Modified`). When it may not be used as it is (stale, or
`no-cache`, or the request's mode asks), and it has one, the request
goes out conditional (`If-None-Match`, else `If-Modified-Since`). A
`304` freshens the stored headers (RFC 9111 4.3.4, all but the
framing ones) and its times, and the stored body answers the request
as a `200`; any other answer replaces it. A request with conditions of
its author's still skips the cache, as Fetch says.

**More of `Cache-Control`.** `must-revalidate` (a stale entry is not
used without revalidation, but by `force-cache` and `only-if-cached`,
which Fetch lets use anything), `immutable` (a fresh entry is not
revalidated by a reload), request directives (`max-age`, `min-fresh`,
`max-stale`, `no-cache`, `no-store`, `only-if-cached`); `s-maxage`
stays a shared cache's. An answer from the cache carries `Age`.

**Fetch's modes, all of them.** `no-cache` now revalidates (it read
nothing before); `default` revalidates a stale entry with validators;
`force-cache` and `only-if-cached` as before; `reload` and `no-store`
skip it.

**Documents too.** A streamed fetch (every document) reads the cache
(a hit is delivered as its head and one piece of body) and writes it (a
cacheable response whose body stays within `MAX_CACHE_ENTRY_BYTES` is
stored when it ends, read aside as it streams). A reload (F5, Ctrl+R)
fetches its document with `no-cache`, its subresources as usual (the
reference browser's reload); a reload that bypasses the cache
(Ctrl+Shift+R, Ctrl+F5, Shift+F5, new) fetches everything with
`reload`; a history traversal not kept by the back-forward cache
fetches its document with `force-cache`.

**Kept in the profile.** `SCHEMA` 5 adds two sealed tables:
`http_cache`, each entry's partition, URL, `Vary` values, times and
headers, read when the profile is opened; and `http_cache_bodies`, its
body, read when the entry is first used. The cache keeps
`MAX_CACHE_BYTES` (64 MiB) of bodies in memory and
`MAX_DISK_CACHE_BYTES` (256 MiB) in the profile, the least recently
used leaving first, and writes what changed at the storage's flush, as
the rest of the profile is written. A private window and
`--temporary-profile` keep it in memory alone. Clearing a site (the
DevTools storage panel, M36) clears its partitions' entries.

### The preload scanner (M38.3)

**In `html`, giving hints, never fetching.** `html::Load` feeds the
text it decodes to a second tokenizer, the scanner, which runs ahead
of the parser (and past a blocking script) and gives back hints: a
stylesheet `<link>`, a `<script src>`, an `<img src>`, a `<link
rel=preload>`, and the first `<base href>`, with the attributes a
request needs (`crossorigin`, `media`, `type`, `nomodule`, `async`,
`referrerpolicy`, `loading`, `as`). It switches its tokenizer's state
for the elements whose text is raw (`script`, `style`, `textarea`,
`title`, `xmp`, `iframe`, `noembed`, `noframes`, `noscript` with
scripting on, `plaintext`) and ignores what is inside `<template>`.
The parsers still never fetch (`docs/architecture.md`).

**In the engine, speculative requests the real ones take.** The
loader turns each hint into a request (its URL resolved against the
base, `media` that does not match and module scripts left out, a lazy
image left out) and fetches it as `Target::Preload`. The page keeps
its preloads by key (the URL, the destination and the CORS mode); a
request the parser makes later for the same key takes the preload's
response, or waits for it, rather than fetching again. A preload no
request took by the `load` event is reported in the console, as the
reference browser does. `<link rel=preload>` in the tree goes through
the same table. In many processes the scanner runs in the renderer and
its requests go to the network service as every other.

### Bounds

`H2_STREAM_WINDOW` (6 MiB), `H2_CONNECTION_WINDOW` (15 MiB),
`MAX_H2_STREAMS` (100, ours, below the server's limit), the peer's
`MAX_HEADER_LIST_SIZE` (256 KiB, and twice that compressed),
`HPACK_TABLE_SIZE` (4096), `MAX_FRAME_SIZE` (16 KiB),
`MAX_H2_CONTROL_FRAMES` (1,000 a second), `MAX_DISK_CACHE_BYTES`
(256 MiB), `MAX_DISK_CACHE_ENTRIES` (10,000), `MAX_PRELOADS` (128 a
document).

### Fuzzing

`hpack` (any bytes decoded: never a panic, the table within its bound;
any header list encoded and decoded the same), `h2_client` (a client
connection fed any server bytes over a few streams: never a panic,
never past a bound, every stream ending), `http_cache` extended to
validation and to its records written and read back, and
`preload_scan` (any HTML: never a panic, every hint's URL an attribute
the document holds).

### The acceptance (M38.4)

A page of twenty resources (its stylesheets, scripts and images, each
`max-age`) loaded, then loaded again in another tab of the profile:
the second makes no request; and again after the profile is closed and
opened, from the disk. The same page from the HTTP/2 test server, each
reply held back a little: every request on one connection, many
streams open at once, the page loaded in about one reply's delay. A
bench of the twenty-resource page over HTTP/1.1 and HTTP/2 with the
server's delay, and of a page whose blocking script holds back what
follows, with and without the scanner.

## 2. Not in this milestone, said rather than hidden

- *HTTP/3 and QUIC* (D15), *h2c* and prior knowledge, *server push*,
  the deprecated `PRIORITY` frames, *extended CONNECT* (WebSockets over
  HTTP/2, which wait for WebSockets, G74), *103 Early Hints*.
- *`stale-while-revalidate`* and `stale-if-error`, a cache size the user
  sets, the cache for methods other than `GET`.
- *`preconnect`, `dns-prefetch`, `prefetch`, `modulepreload`*,
  speculation rules, prerendering; `srcset` and `<picture>`, which the
  engine does not have.
- *Brotli*: `br` is not offered and not decoded, as before.

## 3. Order of work

1. *HPACK and frames*: `net::h2::{hpack, frame}`, with RFC 7541's
   examples; the `hpack` fuzz target.
2. *The HTTP/2 client*: ALPN, the connection thread, streams, flow
   control, settings, errors and retries, the pool; the test server's
   HTTP/2; the protocol in the network log.
3. *Shared and coalesced*: the transport in the context, connection
   coalescing, priorities; the `h2_client` fuzz target.
4. *Revalidation*: validators, `304`, the directives, every mode,
   documents through the cache, reload and traversal.
5. *The cache on disk*: `SCHEMA` 5.
6. *The preload scanner* and `<link rel=preload>`.
7. *The acceptance* and the benches.
8. *T4.3 and fuzzing.*
9. *Documents.*

## 4. Corrected to the code

Filled in at the last step.
