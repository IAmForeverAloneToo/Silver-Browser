# Web Crypto (G255)

Status: ruled 2026-11-22. **aws-lc-rs everywhere, one backend** (the
CNG rule reversed: its aim, no crypto written here, is met better by a
FIPS-validated library already in the tree, no new unsafe, every CI
platform). ChaCha20-Poly1305, ML-KEM and ML-DSA are kept where aws-lc-rs's
stable API has them; what only its unstable features have throws
NotSupportedError, the gap recorded here (no G item). The aws-lc-rs
version pinned, its enabled features recorded here, cargo-deny clean.
Wycheproof (C2SP/wycheproof) and WPT `WebCryptoAPI/` approved, each at a
pinned commit recorded with its hash; Wycheproof runs in CI for every
algorithm exposed. Keys in IndexedDB sealed at rest. Sequence: after G74
part 4, first the pushes that pass Proton's gate and load-time self-test.
Section 2's CNG table stays as the record of why. A beta item: Proton
(the user's mail) refuses a browser without `crypto.subtle`, and 1Password
needs it (docs/design/extensions.md, build step 5).

## 1. What the reference ships (Chrome 157.0.8103.80, probed 2026-11-22)

Probed with pages served from localhost (a secure context), every
algorithm generated, imported and used, and about forty errors made.

`SubtleCrypto.prototype`, in Chrome's order: decrypt, deriveBits,
deriveKey, digest, encrypt, exportKey, generateKey, importKey, sign,
unwrapKey, verify, wrapKey, decapsulateBits, decapsulateKey,
encapsulateBits, encapsulateKey, getPublicKey. `CryptoKey.prototype`:
type, extractable, algorithm, usages. Both constructors throw "Illegal
constructor".

| Family | Chrome 157 has | Not (NotSupportedError "Algorithm: Unrecognized name") |
|---|---|---|
| Digest | SHA-1, SHA-256, SHA-384, SHA-512 | SHA3-*, cSHAKE, TurboSHAKE, MD5 |
| RSA | RSASSA-PKCS1-v1_5, RSA-PSS, RSA-OAEP | |
| EC | ECDSA and ECDH on P-256, P-384, P-521 | |
| Edwards/Montgomery | Ed25519, X25519 | Ed448, X448 |
| AES | AES-CTR, AES-CBC, AES-GCM, AES-KW (128 and 256; 192 refused: "192-bit AES keys are not supported") | AES-OCB |
| AEAD | ChaCha20-Poly1305 (format `raw-secret`, not `raw`) | |
| MAC | HMAC | KMAC |
| KDF | HKDF, PBKDF2 | Argon2id |
| Post-quantum | ML-KEM-768, ML-KEM-1024, ML-DSA-44/65/87 | ML-KEM-512, SLH-DSA |

The probe recorded about forty error cases with Chrome's exact name and
message (for example `SyntaxError: Usages cannot be empty when creating a
key.`, `InvalidAccessError: key.algorithm does not match that of
operation`, `DataError: The required JWK member "k" was missing`,
`OperationError: ` with an empty message for a failed AES-GCM tag). The
implementation takes its messages from that table; the tests hold it.

## 2. Primitives: Windows CNG, measured on this machine

CNG (bcrypt.dll), Windows 10 22H2 (10.0.19045), each algorithm provider
opened and each property set (a PowerShell P/Invoke probe, 2026-11-22):

| Needed | CNG on Windows 10 |
|---|---|
| SHA-1/256/384/512, HMAC | yes |
| AES-GCM, AES-CBC (and CFB, ECB, CCM) | yes |
| AES-KW (RFC 3394) | yes, as a key blob export (`BCRYPT_AES_WRAP_KEY_BLOB`) |
| AES-CTR | **no** chaining mode (`ChainingModeCTR` refused) |
| RSA PKCS#1 v1.5, PSS, OAEP, key generation | yes |
| ECDSA, ECDH on P-256/384/521 | yes |
| X25519 | yes (ECDH with the `curve25519` curve) |
| Ed25519 | **no** (no EdDSA provider; ECDSA refuses curve25519) |
| HKDF, PBKDF2 | yes |
| ChaCha20-Poly1305 | **no** (Windows 11 and Server 2022 have it) |
| ML-KEM, ML-DSA | **no** (newer Windows 11 alone) |

Gaps to decide (yours), each either left out (NotSupportedError, as Chrome
says for a name it lacks) or filled by a vetted crate:
- **Ed25519**: needed. Proton's OpenPGP worker names it ten times (v6 and
  Curve25519 keys); 1Password's list has it. Proposed: aws-lc-rs, already
  in the tree (rustls's provider; the profile's AES-GCM sealing and CSP's
  digests use it now), FIPS-validated, no new dependency.
- **AES-CTR**: Proton's OpenPGP names it (OpenPGP's CFB and EAX are built
  on it). CTR over CNG's ECB would be a mode written here, which the
  ruling forbids; proposed: aws-lc-rs.
- **ChaCha20-Poly1305, ML-KEM, ML-DSA**: no daily site found using them.
  Proposed: left out at first (NotSupportedError), a G item each; or
  aws-lc-rs (ML-KEM is stable there, ML-DSA behind its unstable flag).

One more decision: CI tests on Linux and macOS, where there is no CNG.
Either Web Crypto exists on Windows alone (`crypto.subtle` undefined
elsewhere, its tests `cfg(windows)`: the Windows CI job and this machine
test it), or aws-lc-rs serves the other platforms, which is two backends
to test. A third option, for the record: aws-lc-rs everywhere, no CNG at
all. It already covers every row above, needs no new unsafe (CNG needs
an FFI layer of about twenty-five calls, new unsafe for your approval),
tests the same on every platform, and is about four days shorter. It is
not what the ruling fixed; it is named so you can weigh it.

## 2b. aws-lc-rs 1.18.1's stable surface (read from its source, 2026-11-23)

Stable, no `unstable` feature needed: digests (SHA-1 as
`SHA1_FOR_LEGACY_USE_ONLY`, SHA-2), HMAC, HKDF, PBKDF2, `aead` (AES-GCM,
ChaCha20-Poly1305), `cipher` (AES-CTR, AES-CBC), `key_wrap` (AES-KW 128
and 256), `signature` (RSA PKCS#1 v1.5 and PSS, ECDSA P-256/384/521,
Ed25519, and ML-DSA-44/65/87, moved out of `unstable`), `agreement`
(ECDH on the NIST curves, X25519), `kem` (ML-KEM-512/768/1024), RSA-OAEP
(`rsa`), PKCS#8 parsing and serialization. So every row of section 1
is covered; nothing needs `unstable`.

Gaps in its API, for a ruling:
- **RSA key generation** offers 2048, 3072, 4096 and 8192 bits and the
  exponent 65537 alone; Chrome takes any `modulusLength` its bounds
  allow and the exponents 3 and 65537. Proposed: those four sizes and
  65537; another throws `OperationError` (a difference, recorded, and
  rare in practice).
- **RSA private key's parts** (`d`, `p`, `q`, `dp`, `dq`, `qi`) have no
  getters, so `exportKey("jwk")` of an RSA private key has no way short
  of parsing the PKCS#8 DER ourselves (ruled out) or aws-lc-sys's C
  calls (new unsafe). Proposed: `NotSupportedError` for that one
  export, recorded; import of an RSA private JWK works
  (`from_components`). Public RSA JWKs, every EC and OKP JWK, and every
  PKCS#8/SPKI export work.

## 3. Design

- **Where keys live.** A `CryptoKey` in script is a shell holding a number;
  its material is in a key store of the agent's in Rust (crate `crypto`,
  new, engine-neutral), never in the JS heap. `exportKey`, `wrapKey` and a
  JWK's `ext` check `extractable` in Rust before any byte leaves; the
  bindings cannot reach material for a non-extractable key.
- **Calls.** `Call::Crypto(Box<CryptoCall>)` like the WebSocket's: the
  bindings convert the dictionaries (Web IDL's normalize-an-algorithm in
  Rust, so any engine's bindings share it); each operation runs on a
  thread of a small pool (RSA key generation takes up to seconds) and its
  promise settles in a task. Errors carry Chrome's names and messages
  (section 1's table).
- **Formats.** raw, raw-secret (ChaCha), JWK, SPKI, PKCS#8. JWK is JSON,
  parsed by the engine's JSON parser and checked member by member. SPKI
  and PKCS#8 are DER: decoded by crypt32's `CryptDecodeObjectEx`
  (`X509_PUBLIC_KEY_INFO`, `PKCS_PRIVATE_KEY_INFO`, an OS service like
  CNG) on the CNG path, or by aws-lc-rs's own parsers; never a DER parser
  written here.
- **Secure contexts only**: `crypto.subtle` and `CryptoKey` absent
  elsewhere, as Chrome. **Workers**: dedicated, shared and service.
- **Structured clone**: a `CryptoKey` clones across `postMessage`
  (probed: Chrome keeps `extractable`), and into IndexedDB, which Proton
  and 1Password use to keep non-extractable keys. Its serialized form is
  the material, stored in the profile (whose values are sealed with
  AES-256-GCM at rest, M33, D27, `crates/profile`); the script still
  never sees it.
- **Randomness**: the host's generator `getRandomValues` already uses.

## 4. What Proton needs, and when

Evidence: the reference's calls on load, every `SubtleCrypto` method
wrapped before the page's scripts (account.proton.me/login and
mail.proton.me, signed out); and Proton's public bundles read (the
public-index, crypto-worker and crypto-worker-api chunks).

1. **The gate.** `public-index` sets `protonSupportedBrowser` only when
   `crypto.subtle` exists (with `ol.reversed`, `Object.fromEntries` and
   `trimStart`, which Silver has). Today Silver shows "Unsupported browser"
   here, and this alone needs no algorithm.
2. **On load** (both pages): `importKey raw AES-GCM`, then `encrypt` and
   `decrypt` AES-GCM (a self-test); and an `importKey jwk` of an RSA key
   with `alg: "RS1"` (RSASSA-PKCS1-v1_5, SHA-1), a support check whose
   failure is reported to Proton's error service.
3. **Sign-in, SRP**: the password hashed by bcrypt in script (no Web
   Crypto), expanded by SHA-512; the SRP arithmetic in script BigInt. So
   `digest` SHA-512 (and SHA-256).
4. **The session kept** (`persisted session`): HKDF (SHA-256) from a
   secret of 16 bytes or more to an AES-GCM key (`deriveKey`), then
   AES-GCM.
5. **The mailbox, OpenPGP** (OpenPGP.js in the crypto worker): Ed25519
   and X25519 (Proton's default keys), ECDH and ECDSA on P-256/384/521
   (older keys), RSASSA-PKCS1-v1_5 (RSA keys), AES-KW (ECDH's key wrap),
   AES-CBC and AES-CTR (OpenPGP's CFB and EAX), AES-GCM, HKDF, HMAC,
   SHA-1/256/384/512. Argon2 (the key's S2K) is WebAssembly, not Web
   Crypto. Which of these OpenPGP.js falls back from to script when Web
   Crypto refuses (it does so for some curves) is to be measured in the
   signed-in scratch profile; until then all count as needed.

So Proton needs, in order: the interface at all; AES-GCM, RSASSA SHA-1
JWK import, SHA-2 digests, HKDF; then for mail Ed25519, X25519, the NIST
curves, RSA, AES-KW/CBC/CTR and HMAC.

## 5. Tests

- Wycheproof vectors (Google's, Apache-2.0) for AES-GCM, AES-CBC, AES-KW,
  ChaCha20-Poly1305 if kept, ECDSA, ECDH, X25519, Ed25519, RSA-PSS/OAEP/
  PKCS1, HKDF, HMAC: a download to approve; the vectors kept as test data.
- WPT `WebCryptoAPI/` (a WPT pull to approve), its expectations at Chrome's.
- Section 1's error table as engine tests, each Chrome's words.
- A fuzz target `crypto_import`: any bytes as raw, SPKI, PKCS#8 and JWK for
  each algorithm; never a panic, and a key imported exports back (when
  extractable) to the bytes given, for raw and the DER forms.
- Sabotage as usual; a test that no operation can hand a non-extractable
  key's bytes to script.

## 6. Estimate

CNG path, with aws-lc-rs for Ed25519 and AES-CTR:

| Part | Days |
|---|---|
| Algorithm normalization, errors, the bindings, promises in tasks | 1.5 |
| CNG layer (FFI, handles), digests, HMAC, AES-GCM/CBC/KW, RSA x3, EC x2, X25519, HKDF, PBKDF2 | 3.0 |
| Formats: raw, JWK, SPKI and PKCS#8 (crypt32) | 1.5 |
| Ed25519, AES-CTR (aws-lc-rs) | 0.5 |
| Key store, structured clone, IndexedDB, workers, secure contexts | 1.0 |
| Tests: Wycheproof, WPT pull and expectations, fuzz target, sabotage | 2.0 |
| Docs, gate, Proton check (the gate passes, login page loads) | 0.5 |
| **Total** | **10** |

The aws-lc-rs-only path: about 6 (no CNG layer or crypt32 formats, one
backend on every platform).

Asks: the CNG layer's new unsafe; the Wycheproof and WPT downloads; the
decisions in section 2.

## 7. Where it stands

First push (Proton's gate and its load-time steps):

- `crates/crypto`, engine-neutral, every primitive aws-lc-rs's (pinned
  `=1.18.1`, features `aws-lc-sys` and `prebuilt-nasm` alone): the key
  store, the normalization with the reference's messages and member
  order (section 1's probes; `name, iv, additionalData, tagLength`,
  `hash` read twice), and SHA-1/256/384/512,
  AES-GCM (96-bit IVs, 128-bit tags), HMAC, HKDF, PBKDF2, and
  RSASSA-PKCS1-v1_5 public keys (JWK import, verify, JWK and SPKI
  export), each export the reference allows.

Second push: `native`, the one module of calls into aws-lc's C library
(aws-lc-sys, pinned `=0.45.0`, the version aws-lc-rs brings), on the
lead's conditions: AES-GCM with any IV and tag length the reference
allows (96 and 128 bits stay aws-lc-rs's), and an RSA public key's
parts (an SPKI read by aws-lc-rs, its PKCS #1 key by the library;
never DER read here). RSASSA-PKCS1-v1_5's verification is routed by
the key's size (the lead's ruling (a)): aws-lc-rs's safe API inside its
ranges (1024 to 8192 bits, 2048 to 8192 for SHA-384), the library's
`RSA_verify` outside them, from 512 bits to 16384 as the reference
verifies (probed: Chrome 157 imports and verifies an openssl key of
16384 bits); a key the library refuses is refused as it is imported.
RSA's key pairs (part 2b-2): generated over the reference's range
(256 to 8192 bits, a multiple of 8, e 3 or 65537, its errors in its
order; none under 512 bits, an OperationError; the size made rounded
down to a multiple of 128 bits and reported so, as Chrome does),
aws-lc-rs's for 2048, 3072, 4096 and 8192 bits at 65537, the library's
for the rest; private keys as PKCS #8 and JWK, imported and exported
(the library parses every PKCS #8, aws-lc-rs's parse required too
inside its range; outside it, the PKCS #8 written here around the
library's PKCS #1, byte for byte aws-lc-rs's inside the range and read
back to the same parts outside it, in each of DER's length forms); and
RSASSA-PKCS1-v1_5's signing, aws-lc-rs's for SHA-256 to SHA-512 at 2048
to 8192 bits, the library's for the rest (SHA-1 among them: aws-lc-rs
signs none). RSA-PSS (part 2b-3), routed by hash, size and salt:
aws-lc-rs's for SHA-256 to SHA-512 at 2048 to 8192 bits with a salt as
long as the digest (the one it takes), the library's for any other
(SHA-1, a salt from 0 to the most the key holds, emLen - hLen - 2);
past that most, an OperationError to sign and false to verify, past
2^31 - 1 an OperationError, as Chrome 157 answers (probed); JWK `alg`
PS1 to PS512. RSA-OAEP (part 2b-4) is aws-lc-rs's alone, for a key of
2048 to 8192 bits: its label given (an empty one is none), the hash's
MGF1, up to k - 2 hLen - 2 bytes, and every failure an OperationError
with no message, as Chrome 157 answers (probed); JWK `alg` RSA-OAEP and
RSA-OAEP-256 to RSA-OAEP-512. A key outside that range is made,
imported and exported, but encrypts and decrypts nothing: an
OperationError saying so, a known gap by the lead's ruling (2026-11-23),
where Chrome works; it reopens if a site the user uses needs it. Next:
Wycheproof's RSA vectors.

On any version bump of aws-lc-rs (or aws-lc-sys), before the push: read
again the spots `native` leans on, `RSA_new_private_key`
(crypto/fipsmodule/rsa/rsa.c) and `RSA_parse_private_key`
(crypto/rsa_extra/rsa_asn1.c), each ending with `RSA_check_key`, which
`native` repeats on purpose and no test can see while they stand, and
`RSA_check_key`'s own `GUARD_PTR` (crypto/fipsmodule/rsa/rsa.c), which
refuses a null key `native` refuses first, as unseen; and
run the native module's sabotage again (the lead's ruling,
2026-11-23). Each C object freed by its wrapper on
every path, every return code checked, every script-controlled length
checked first, the tag checked by the library.
- The bindings a shell (`SubtleCrypto`, `CryptoKey`), `crypto.subtle` in
  secure contexts and workers alone; a call's promise settles in a task
  (`Task::Crypto`); a key is its number in script.
- Tests: the RFCs' vectors (SHA, HKDF 5869, PBKDF2 6070, HMAC 4231), the
  reference's messages, Proton's own steps in a page (its AES-GCM
  self-test, its RS1 key, HKDF to AES-GCM), a worker, an insecure
  context; the fuzz target `crypto_import` (raw, SPKI and JWK, any
  bytes, an extractable key's export importing again as itself).

To come: AES-CBC/CTR/KW, RSA-PSS/OAEP and RSA private keys, ECDSA, ECDH,
Ed25519, X25519, ChaCha20-Poly1305, ML-KEM, ML-DSA, wrapKey/unwrapKey,
a `CryptoKey`'s structured clone (postMessage, IndexedDB), Wycheproof's
vectors and WPT's `WebCryptoAPI/`. Rulings asked: AES-GCM's other IV and
tag lengths and RSA's other key sizes and private JWK export (section
2b).
