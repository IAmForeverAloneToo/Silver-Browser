# Chrome's surface, copied on purpose

Silver claims to be Chrome, and a site sees what Chrome shows it. Where
Chrome differs from a spec, or is plainly wrong, and a site could tell,
Silver does as Chrome does. This file lists each such copy, one section
per API: what Chrome does, what the spec (or WPT) says instead, the probe
that showed it, and the Chrome version it ran on.

Rules:

- Every deliberate Chrome deviation gets a line here in the same commit
  that makes it. A difference from Chrome that is not deliberate is a
  bug, or a gap with a G number; it does not belong here.
- The probes are in `tools/chrome-probes/` (its README says how to run
  them). Each output records the version it ran on. The reference is the
  installed Chrome stable: when it moves, re-probe the lines below
  before relying on them, and update the version here.
- A WPT failure kept because of a line here says so in its expectation
  reason.
- Where Silver differs from Chrome on purpose instead, the line goes
  under "We differ, deliberately", at the end, with the same probe and
  version, and why.

## User-Agent and client hints

Probe: `tools/chrome-probes/ua/` (`ua_server.py`; `page.json`,
`headers.jsonl`), Chrome 158.0.8127.37.

- `User-Agent` and `navigator.userAgent` are Chrome's reduced string on
  the same system, letter for letter: `Mozilla/5.0 (Windows NT 10.0;
  Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/158.0.0.0
  Safari/537.36` (the probe runs Chrome headless, which says
  `HeadlessChrome`; headed Chrome says `Chrome`). There is no Silver
  token in it: a token no Chrome
  sends is what a site's check sees (D80's `Silver/<version>` token is
  void). The honest minimal string (D12) is sent only when the user
  turns the compatible one off. (`net::fetch::USER_AGENT`.)
- The brands (`navigator.userAgentData.brands`, `Sec-CH-UA`) are
  Chromium's list for the major version, in Chromium's order and GREASE
  form, with Silver's brand where Chrome has "Google Chrome", as Brave
  names itself: for 158, "Chromium" 158, "Not.A/Brand" 24, "Silver"
  158. The full version list gives 158.0.8127.37 and the GREASE brand's
  24.0.0.0. (`net::client_hints`; test
  `the_brands_are_chromiums_for_the_major_version`.)

## Web Crypto

Probes: `tools/chrome-probes/webcrypto/`, Chrome 158.0.8127.37. Design:
`docs/design/webcrypto.md`.

- ChaCha20-Poly1305: an IV that is not 12 bytes is refused with an
  OperationError whose message says `The "iv" has an unexpected length
  -- must be 16 bytes`. The algorithm takes 12 bytes; 16 is AES-CBC's.
  Chrome's message is plainly wrong, and copied word for word
  (`chacha.html`; `crypto::algorithms::chacha`).
- `deriveBits`' `length` is converted by ToUint32, wrapped and never
  refused for its range: -8 is 2^32 - 8 bits, 2^32 + 8 is 8, NaN and
  the infinities are 0. WPT's `derived_bits_length` expects a TypeError
  for -8 (the length taken with EnforceRange). Silver does as Chrome
  does; those WPT pages are `not-run` until G255 step 7 makes the long
  operations cancellable (`enforce-range.html`).
- `QuotaExceededError` (the interface): its constructor takes a negative
  `quota`, a negative `requested` and a `requested` below `quota` as
  given, where the spec throws a RangeError; only a non-finite one is a
  TypeError, as the dictionary's `double` makes it. A clone keeps the
  class and both fields (`quota-iface.html`; test
  `subtle_crypto_supports_and_quota_exceeded_error_are_the_references`).
- `getRandomValues` over 65,536 bytes throws a `QuotaExceededError` whose
  `quota` and `requested` are both null, not 65536 and the length asked.
  This agrees with WPT's `getRandomValues.any.js`, which expects null
  for both; it is listed so that nobody "fixes" it to the numbers
  (`quota.html`).

## We differ, deliberately

### Web Crypto

- `SubtleCrypto.supports('deriveBits', hkdf, -8)`: Chrome says true,
  Silver false. -8 wraps to 2^32 - 8 bits, past HKDF's 255 hash
  lengths, and Chrome's own `deriveBits` refuses it ("The length
  provided for HKDF is too large."); `supports` must never say yes to
  what the operation refuses (the lead's ruling, G255), and WPT's
  `supports.tentative` expects false. The same holds for any HKDF
  length past 255 hash lengths (`supports.html`, `enforce-range.html`;
  Chrome 158.0.8127.37).
