//! Web Crypto in a page and a worker (G255): Proton's load-time steps
//! first (the gate, its AES-GCM self-test, its RSASSA SHA-1 check, its
//! persisted session's HKDF), each answer and error what the reference
//! browser gave (Chrome 157, probed 2026-11-22/23); and none of it in an
//! insecure context.

use crate::testing::*;

/// The lines a page of `script` logs (a secure one: a loopback host's),
/// until `count` have come.
fn logged(script: &str, count: usize) -> Vec<String> {
    let page = format!("<script>{script}</script>");
    let server = serve(move |path| (path == "/page.html").then(|| html(&page)));
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/page.html"));
    tab.run_until(|lines| lines.len() >= count)
}

const PROTON_RS1: &str = "{alg: 'RS1', e: 'AQAB', kty: 'RSA', n: 'rVMOV0fT7doXOgDU-drrk9yXIgEsB9aK5x9fSOpRuWa8aS9hmX9obPoOub1Boz0Rk_C4maZMKo8m7mHx3n6JXQ'}";

#[test]
fn a_secure_page_has_subtle_crypto_and_protons_steps_pass() {
    let script = format!(
        "(async () => {{\
         const s = crypto.subtle; const out = [];\
         const hex = (b) => Array.from(new Uint8Array(b), (x) => x.toString(16).padStart(2, '0')).join('');\
         const json = (v) => JSON.stringify(v, (k, x) => x instanceof Uint8Array ? Array.from(x) : x);\
         out.push([typeof s, Object.prototype.toString.call(s), s === crypto.subtle].join(' '));\
         out.push(Object.getOwnPropertyNames(Crypto.prototype).join());\
         out.push(Object.getOwnPropertyNames(CryptoKey.prototype).join());\
         out.push(Object.getOwnPropertyNames(SubtleCrypto.prototype).slice(0, 12).join());\
         out.push(hex(await s.digest('SHA-256', new TextEncoder().encode('abc'))));\
         const aes = await s.importKey('raw', new Uint8Array(32), 'AES-GCM', false, ['decrypt', 'encrypt']);\
         const iv = new Uint8Array(12);\
         const sealed = await s.encrypt({{name: 'AES-GCM', iv}}, aes, new TextEncoder().encode('proton'));\
         out.push(new TextDecoder().decode(await s.decrypt({{name: 'AES-GCM', iv}}, aes, sealed)) + ' ' + sealed.byteLength + ' ' + aes.usages.join());\
         const rs1 = await s.importKey('jwk', {PROTON_RS1}, {{name: 'RSASSA-PKCS1-v1_5', hash: {{name: 'SHA-1'}}}}, true, ['verify']);\
         out.push(rs1.type + ' ' + json(rs1.algorithm) + ' ' + (rs1.algorithm === rs1.algorithm) + ' ' + String(rs1));\
         const secret = await s.importKey('raw', new Uint8Array(32).fill(7), 'HKDF', false, ['deriveKey']);\
         const derived = await s.deriveKey({{name: 'HKDF', hash: 'SHA-256', salt: new Uint8Array(16), info: new Uint8Array(0)}}, secret, {{name: 'AES-GCM', length: 256}}, false, ['encrypt', 'decrypt']);\
         out.push(derived.type + ' ' + json(derived.algorithm) + ' ' + derived.extractable);\
         const err = async (f) => {{ try {{ await f(); return 'no error'; }} catch (e) {{ return e.name + ': ' + e.message; }} }};\
         out.push(await err(() => s.encrypt({{name: 'AES-GCM'}}, aes, new Uint8Array(1))));\
         out.push(await err(() => s.exportKey('raw', aes)));\
         out.push(await err(() => s.digest()));\
         out.push(await err(() => s.digest('MD5', new Uint8Array(1))));\
         const gen = await s.generateKey({{name: 'AES-GCM', length: 128}}, true, ['encrypt']);\
         out.push(Object.keys(await s.exportKey('jwk', gen)).join());\
         for (const line of out) console.log(line);\
         }})().catch((e) => console.log('failed', e.name, e.message));"
    );
    let lines = logged(&script, 13);
    assert_eq!(
        lines,
        [
            "object [object SubtleCrypto] true",
            "getRandomValues,constructor,subtle,randomUUID",
            "type,extractable,algorithm,usages,constructor",
            "decrypt,deriveBits,deriveKey,digest,encrypt,exportKey,generateKey,importKey,sign,unwrapKey,verify,wrapKey",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            "proton 22 encrypt,decrypt",
            "public {\"name\":\"RSASSA-PKCS1-v1_5\",\"hash\":{\"name\":\"SHA-1\"},\"modulusLength\":512,\"publicExponent\":[1,0,1]} true [object CryptoKey]",
            "secret {\"name\":\"AES-GCM\",\"length\":256} false",
            "TypeError: Failed to execute 'encrypt' on 'SubtleCrypto': AeadParams: iv: Missing required property",
            "InvalidAccessError: Failed to execute 'exportKey' on 'SubtleCrypto': key is not extractable",
            "TypeError: Failed to execute 'digest' on 'SubtleCrypto': 2 arguments required, but only 0 present.",
            "NotSupportedError: Failed to execute 'digest' on 'SubtleCrypto': Algorithm: Unrecognized name",
            "alg,ext,k,key_ops,kty",
        ]
    );
}

#[test]
fn a_dedicated_worker_has_it_too() {
    let worker = "crypto.subtle.digest('SHA-1', new TextEncoder().encode('abc')).then((d) =>\
        postMessage(Array.from(new Uint8Array(d), (x) => x.toString(16).padStart(2, '0')).join('')));";
    let page = format!(
        "<script>const w = new Worker(URL.createObjectURL(new Blob([\"{worker}\"])));\
         w.onmessage = (e) => console.log(e.data);</script>"
    );
    let server = serve(move |path| (path == "/page.html").then(|| html(&page)));
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/page.html"));
    let lines = tab.run_until(|lines| !lines.is_empty());
    assert_eq!(lines, ["a9993e364706816aba3e25717850c26c9cd0d89d"]);
}

#[test]
fn an_insecure_context_has_none_of_it() {
    let server = serve(|path| {
        (path == "/page.html").then(|| {
            html(
                "<script>console.log([isSecureContext, typeof crypto.subtle, 'subtle' in crypto, \
                 typeof SubtleCrypto, typeof CryptoKey, typeof crypto.getRandomValues].join(' '));</script>",
            )
        })
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.set_network(net::Client::new(
        net::transport::Config::default()
            .resolve("insecure.test", std::net::IpAddr::from([127, 0, 0, 1])),
    ));
    let url =
        net::Url::parse(&format!("http://insecure.test:{}/page.html", server.port())).unwrap();
    tab.engine.navigate(url);
    let lines = tab.run_until(|lines| !lines.is_empty());
    assert_eq!(
        lines,
        ["false undefined false undefined undefined function"]
    );
}

/// An RSA key pair through a page's own calls (G255 part 2b): made at a
/// size outside aws-lc-rs's range and inside it, each signing what its
/// public key verifies, its private key exported and imported again;
/// each answer what the reference gave (Chrome 157, probed 2026-11-23).
#[test]
fn a_page_makes_an_rsa_pair_signs_verifies_and_moves_its_keys() {
    let script = "(async () => {\
        const s = crypto.subtle; const out = [];\
        const alg = {name: 'RSASSA-PKCS1-v1_5', hash: 'SHA-256'};\
        const data = new TextEncoder().encode('pair');\
        for (const bits of [1016, 2048]) {\
          const pair = await s.generateKey({...alg, modulusLength: bits, publicExponent: new Uint8Array([1, 0, 1])}, true, ['sign', 'verify']);\
          const sig = await s.sign(alg, pair.privateKey, data);\
          const pk8 = await s.exportKey('pkcs8', pair.privateKey);\
          const again = await s.importKey('pkcs8', pk8, alg, false, ['sign']);\
          const sig2 = await s.sign(alg, again, data);\
          const jwk = await s.exportKey('jwk', pair.privateKey);\
          out.push([Object.prototype.toString.call(pair), Object.keys(pair).join(), pair.publicKey.type, pair.privateKey.type,\
            pair.publicKey.usages.join(), pair.privateKey.usages.join(), pair.privateKey.algorithm.modulusLength, sig.byteLength,\
            await s.verify(alg, pair.publicKey, sig, data), await s.verify(alg, pair.publicKey, sig2, data),\
            again.extractable, Object.keys(jwk).join()].join(' '));\
        }\
        const err = async (f) => { try { await f(); return 'no error'; } catch (e) { return e.name + ': ' + e.message; } };\
        const pair = await s.generateKey({...alg, modulusLength: 512, publicExponent: new Uint8Array([3])}, true, ['sign', 'verify']);\
        out.push(await err(() => s.exportKey('spki', pair.privateKey)));\
        out.push(await err(() => s.sign(alg, pair.publicKey, data)));\
        out.push(await err(() => s.generateKey({...alg, modulusLength: 1023, publicExponent: new Uint8Array([1, 0, 1])}, true, ['sign'])));\
        for (const line of out) console.log(line);\
        })().catch((e) => console.log('failed', e.name, e.message));";
    let lines = logged(script, 5);
    assert_eq!(
        lines,
        [
            "[object Object] privateKey,publicKey public private verify sign 896 112 true true false alg,d,dp,dq,e,ext,key_ops,kty,n,p,q,qi",
            "[object Object] privateKey,publicKey public private verify sign 2048 256 true true false alg,d,dp,dq,e,ext,key_ops,kty,n,p,q,qi",
            "InvalidAccessError: The key is not of the expected type",
            "InvalidAccessError: key.usages does not permit this operation",
            "OperationError: The modulus length must be a multiple of 8 bits and >= 256 and <= 8192",
        ]
    );
}

/// wrapKey and unwrapKey through a page's own calls (G255 step 1): a key
/// wrapped by AES-KW and as a JWK by AES-GCM, unwrapped again and used;
/// their arguments' errors and AES-CTR's, the reference's (Chrome 157,
/// probed 2026-11-23).
#[test]
fn a_page_wraps_and_unwraps_keys() {
    let script = "(async () => {\
        const s = crypto.subtle; const out = [];\
        const hex = (b) => Array.from(new Uint8Array(b), (x) => x.toString(16).padStart(2, '0')).join('');\
        const k16 = new Uint8Array(16).map((_, i) => i);\
        const kw = await s.importKey('raw', k16, 'AES-KW', false, ['wrapKey', 'unwrapKey']);\
        const hmac = {name: 'HMAC', hash: 'SHA-256'};\
        const secret = await s.importKey('raw', new Uint8Array(32).fill(5), hmac, true, ['sign', 'verify']);\
        const wrapped = await s.wrapKey('raw', secret, kw, 'AES-KW');\
        out.push(hex(wrapped));\
        const back = await s.unwrapKey('raw', wrapped, kw, 'AES-KW', hmac, false, ['sign']);\
        out.push([back.type, back.extractable, back.usages.join(), back.algorithm.name, back.algorithm.length].join(' '));\
        const gcm = await s.importKey('raw', k16, 'AES-GCM', false, ['wrapKey', 'unwrapKey']);\
        const p = {name: 'AES-GCM', iv: new Uint8Array(12)};\
        const viaJwk = await s.unwrapKey('jwk', await s.wrapKey('jwk', secret, gcm, p), gcm, p, hmac, true, ['verify']);\
        const sig = await s.sign('HMAC', back, new Uint8Array([1]));\
        out.push(String(await s.verify('HMAC', viaJwk, sig, new Uint8Array([1]))));\
        const err = async (f) => { try { await f(); return 'no error'; } catch (e) { return e.name + ': ' + e.message; } };\
        out.push(await err(() => s.wrapKey('raw', secret, kw)));\
        out.push(await err(() => s.wrapKey('raw', secret, secret, 'AES-KW')));\
        out.push(await err(() => s.unwrapKey('raw', wrapped, kw, 'AES-KW', hmac, true, ['encrypt'])));\
        const ctr = await s.importKey('raw', k16, 'AES-CTR', false, ['encrypt']);\
        out.push(hex(await s.encrypt({name: 'AES-CTR', counter: new Uint8Array(16).fill(9), length: 1}, ctr, new Uint8Array(20).fill(2))));\
        for (const line of out) console.log(line);\
        })().catch((e) => console.log('failed', e.name, e.message));";
    let lines = logged(script, 7);
    assert_eq!(
        lines,
        [
            "f5e7c56fcfb7fe11e79873433832d0bc0993fafc624753d4a4123705041be60f661214323ec097ee",
            "secret false sign HMAC 256",
            "true",
            "TypeError: Failed to execute 'wrapKey' on 'SubtleCrypto': 4 arguments required, but only 3 present.",
            "InvalidAccessError: key.algorithm does not match that of operation",
            "SyntaxError: Cannot create a key using the specified key usages.",
            "ee68613a0882e37671c9580dd2eb985661aea1fa",
        ]
    );
}

/// ECDSA and ECDH through a page's own calls (G255 step 2): a signature
/// verifies; both sides of ECDH derive the same bits, the `public`
/// member a CryptoKey the bindings hand over by its number.
#[test]
fn a_page_signs_with_ecdsa_and_agrees_with_ecdh() {
    let script = "(async () => {\
        const s = crypto.subtle; const out = [];\
        const p = await s.generateKey({name: 'ECDSA', namedCurve: 'P-384'}, false, ['sign', 'verify']);\
        const alg = {name: 'ECDSA', hash: 'SHA-384'};\
        const sig = await s.sign(alg, p.privateKey, new Uint8Array([1]));\
        out.push(sig.byteLength + ' ' + await s.verify(alg, p.publicKey, sig, new Uint8Array([1])));\
        const a = await s.generateKey({name: 'ECDH', namedCurve: 'P-256'}, false, ['deriveBits']);\
        const b = await s.generateKey({name: 'ECDH', namedCurve: 'P-256'}, false, ['deriveBits']);\
        const x = new Uint8Array(await s.deriveBits({name: 'ECDH', public: b.publicKey}, a.privateKey, 256));\
        const y = new Uint8Array(await s.deriveBits({name: 'ECDH', public: a.publicKey}, b.privateKey, 256));\
        out.push(x.length + ' ' + x.every((v, i) => v === y[i]));\
        const err = async (f) => { try { await f(); return 'no error'; } catch (e) { return e.name + ': ' + e.message; } };\
        out.push(await err(() => s.deriveBits({name: 'ECDH', public: 5}, a.privateKey, 256)));\
        for (const line of out) console.log(line);\
        })().catch((e) => console.log('failed', e.name, e.message));";
    let lines = logged(script, 3);
    assert_eq!(
        lines,
        [
            "96 true",
            "32 true",
            "TypeError: Failed to execute 'deriveBits' on 'SubtleCrypto': EcdhKeyDeriveParams: public: Missing or not a CryptoKey",
        ]
    );
}

/// A CryptoKey's structured clone through a page (G255 step 4): by
/// structuredClone, to a worker and back by postMessage (the worker
/// signing with the key it was sent), and through IndexedDB; each copy a
/// new CryptoKey with the same type, algorithm, extractable and usages,
/// and still not extractable where the key was not.
#[test]
fn a_page_clones_crypto_keys_to_a_worker_and_through_indexeddb() {
    let script = "(async () => {\
        const s = crypto.subtle; const out = [];\
        const pair = await s.generateKey({name: 'ECDSA', namedCurve: 'P-256'}, false, ['sign', 'verify']);\
        const alg = {name: 'ECDSA', hash: 'SHA-256'};\
        const copy = structuredClone(pair.privateKey);\
        out.push([copy !== pair.privateKey, copy instanceof CryptoKey, copy.type, copy.extractable, copy.usages.join(), copy.algorithm.namedCurve].join(' '));\
        const code = \"onmessage = async (e) => { const sig = await crypto.subtle.sign({name: 'ECDSA', hash: 'SHA-256'}, e.data, new Uint8Array([7])); postMessage([sig, e.data]); };\";\
        const w = new Worker(URL.createObjectURL(new Blob([code], {type: 'text/javascript'})));\
        const [sig, back] = await new Promise((resolve) => { w.onmessage = (e) => resolve(e.data); w.postMessage(pair.privateKey); });\
        out.push(await s.verify(alg, pair.publicKey, sig, new Uint8Array([7])) + ' ' + back.type + ' ' + back.extractable);\
        const db = await new Promise((resolve, reject) => { const r = indexedDB.open('keys', 1); r.onupgradeneeded = () => r.result.createObjectStore('k'); r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error); });\
        await new Promise((resolve) => { const tx = db.transaction('k', 'readwrite'); tx.objectStore('k').put({key: pair.privateKey}, 1); tx.oncomplete = resolve; });\
        const stored = await new Promise((resolve) => { const r = db.transaction('k').objectStore('k').get(1); r.onsuccess = () => resolve(r.result.key); });\
        const sig2 = await s.sign(alg, stored, new Uint8Array([8]));\
        out.push(await s.verify(alg, pair.publicKey, sig2, new Uint8Array([8])) + ' ' + stored.extractable);\
        try { await s.exportKey('pkcs8', stored); out.push('exported'); } catch (e) { out.push(e.name); }\
        for (const line of out) console.log(line);\
        })().catch((e) => console.log('failed', e.name, e.message));";
    let lines = logged(script, 4);
    assert_eq!(
        lines,
        [
            "true true private false sign P-256",
            "true private false",
            "true false",
            "InvalidAccessError",
        ]
    );
}

/// What the bindings read as the reference reads it (G255 step 6, Chrome
/// 157 probed: undef-member, dict-int; WPT's "during call" tests): a
/// dictionary member there as undefined is converted, one not there is
/// missing, and an inherited one is there; a method's data is copied
/// once its algorithm is normalized, so that an algorithm's getter that
/// transfers or alters it is seen; and getRandomValues' QuotaExceededError
/// has no quota or amount.
#[test]
fn the_bindings_read_arguments_in_the_references_order() {
    let script = "(async () => {\
        const s = crypto.subtle; const out = [];\
        const t = async (f) => { try { out.push(String(await f())); } catch (e) { out.push(e.name + ': ' + e.message); } };\
        await t(async () => (await s.generateKey({name: 'AES-GCM', length: undefined}, true, ['encrypt'])).algorithm.length);\
        await t(async () => (await s.generateKey({name: 'AES-GCM'}, true, ['encrypt'])).algorithm.length);\
        const inherited = Object.create({length: 128}); inherited.name = 'AES-GCM';\
        await t(async () => (await s.generateKey(inherited, true, ['encrypt'])).algorithm.length);\
        await t(async () => (await s.generateKey({name: 'AES-GCM', length: NaN}, true, ['encrypt'])).algorithm.length);\
        const key = await s.generateKey({name: 'AES-GCM', length: 128}, true, ['encrypt']);\
        await t(async () => (await s.encrypt({name: 'AES-GCM', iv: undefined}, key, new Uint8Array(1))).byteLength);\
        const data = new Uint8Array(480);\
        const moving = {name: 'AES-GCM', get iv() { structuredClone(data.buffer, {transfer: [data.buffer]}); return new Uint8Array(12); }};\
        await t(async () => (await s.encrypt(moving, key, data)).byteLength);\
        const text = new Uint8Array([1, 2, 3]);\
        const altering = {get name() { text[0] = 9; return 'SHA-256'; }};\
        const hex = (b) => Array.from(new Uint8Array(b), (x) => x.toString(16).padStart(2, '0')).join('');\
        const altered = hex(await s.digest(altering, text));\
        out.push(String(altered === hex(await s.digest('SHA-256', new Uint8Array([9, 2, 3])))));\
        try { crypto.getRandomValues(new Uint8Array(65537)); } catch (e) { out.push([e instanceof QuotaExceededError, e.quota, e.requested].join(' ')); }\
        for (const line of out) console.log(line);\
        })().catch((e) => console.log('failed', e.name, e.message));";
    let lines = logged(script, 8);
    assert_eq!(
        lines,
        [
            "TypeError: Failed to execute 'generateKey' on 'SubtleCrypto': AesKeyGenParams: length: Is not a number",
            "TypeError: Failed to execute 'generateKey' on 'SubtleCrypto': AesKeyGenParams: length: Missing required property",
            "128",
            "TypeError: Failed to execute 'generateKey' on 'SubtleCrypto': AesKeyGenParams: length: Is not a number",
            "TypeError: Failed to execute 'encrypt' on 'SubtleCrypto': AeadParams: iv: Not a BufferSource",
            "16",
            "true",
            "true  ",
        ]
    );
}

/// SubtleCrypto.supports as the reference has it (Chrome 157, probed:
/// supports): a static method of two arguments; an operation's name as
/// it is; the last argument a wrapping length, or deriveKey's derived
/// key type; and QuotaExceededError's interface (probed: quota-iface).
#[test]
fn subtle_crypto_supports_and_quota_exceeded_error_are_the_references() {
    let script = "(async () => {\
        const S = SubtleCrypto; const out = [];\
        const t = (f) => { try { out.push(String(f())); } catch (e) { out.push(e.name + ': ' + e.message); } };\
        const d = Object.getOwnPropertyDescriptor(S, 'supports');\
        t(() => [typeof S.supports, S.supports.length, S.supports.name, d.writable, d.enumerable, d.configurable, 'supports' in S.prototype].join());\
        t(() => S.supports('digest'));\
        const hkdf = {name: 'HKDF', hash: 'SHA-256', salt: new Uint8Array(1), info: new Uint8Array(0)};\
        t(() => [S.supports('digest', 'sha-256'), S.supports('GENERATEKEY', 'AES-GCM'), S.supports('generateKey', {name: 'AES-GCM', length: 192}), S.supports('generateKey', {name: 'AES-GCM', length: 128.5})].join());\
        t(() => [S.supports('deriveBits', hkdf, 256), S.supports('deriveBits', hkdf, -8), S.supports('deriveBits', hkdf, '256'), S.supports('deriveBits', hkdf), S.supports('deriveBits', hkdf, 230)].join());\
        t(() => [S.supports('deriveKey', hkdf, {name: 'AES-GCM', length: 256}), S.supports('deriveKey', hkdf, {name: 'AES-GCM', length: 192}), S.supports('deriveKey', hkdf)].join());\
        t(() => [S.supports('encrypt', {name: 'AES-CTR', counter: new Uint8Array(16), length: 129}), S.supports('exportKey', 'AES-GCM'), S.supports(5, 'SHA-256')].join());\
        t(() => [S.supports('wrapKey', 'AES-KW', 'AES-GCM'), S.supports('wrapKey', 'AES-KW', 'HKDF'), S.supports('unwrapKey', 'AES-KW', 'HKDF'), S.supports('exportKey', 'HKDF'), S.supports('importKey', {name: 'ECDSA', namedCurve: 'P-51'})].join());\
        const ecdh = await crypto.subtle.generateKey({name: 'ECDH', namedCurve: 'P-256'}, false, ['deriveBits']);\
        t(() => [S.supports('deriveBits', {name: 'ECDH', public: ecdh.publicKey}, 256), S.supports('deriveBits', {name: 'ECDH', public: ecdh.publicKey}, 257), S.supports('deriveBits', {name: 'ECDH', public: ecdh.privateKey}, 256), S.supports('deriveKey', {name: 'ECDH', public: ecdh.publicKey}, {name: 'HMAC', hash: 'SHA-256'})].join());\
        const Q = QuotaExceededError;\
        t(() => [Q.length, Object.getPrototypeOf(Q) === DOMException, Object.getOwnPropertyNames(Q.prototype).join('/')].join());\
        t(() => { const e = new Q('m', {quota: 5, requested: 2}); return [e.quota, e.requested, e.code, e instanceof DOMException].join(); });\
        t(() => new Q('m', {quota: Infinity}));\
        t(() => { const c = structuredClone(new Q('m', {quota: 5, requested: 7})); return [c.constructor === Q, c.message, c.quota, c.requested].join(); });\
        t(() => { const c = structuredClone(new Q('m')); return [c.constructor === Q, c.quota, c.requested].join(); });\
        for (const line of out) console.log(line);\
        })().catch((e) => console.log('failed', e.name, e.message));";
    let lines = logged(script, 13);
    assert_eq!(
        lines,
        [
            "function,2,supports,true,true,true,false",
            "TypeError: Failed to execute 'supports' on 'SubtleCrypto': 2 arguments required, but only 1 present.",
            "true,false,false,true",
            // -8 wraps to 2^32 - 8 bits, past HKDF's most: deriveBits
            // refuses it (the reference's too), so supports says no,
            // where the reference's says yes (the contract over its
            // answer; WPT's supports.tentative expects no).
            "true,false,false,false,false",
            "true,false,false",
            "false,true,false",
            "true,false,true,false,false",
            "true,false,false,false",
            "0,true,quota/requested/constructor",
            "5,2,22,true",
            "TypeError: Failed to construct 'QuotaExceededError': Failed to read the 'quota' property from 'QuotaExceededErrorOptions': The provided double value is non-finite.",
            "true,m,5,7",
            "true,,",
        ]
    );
}

/// A value as a page's script writes it: `None` for a key, which only
/// the store's numbers name.
fn js(value: &crypto::Value) -> Option<String> {
    use crypto::Value;
    let text = |s: &str| devtools::Json::from(s.to_owned()).write();
    Some(match value {
        Value::Absent => "undefined".to_owned(),
        Value::Str(s) => text(s),
        Value::Num(n) => format!("{n}"),
        Value::Bool(b) => b.to_string(),
        Value::Bytes(bytes) => format!(
            "new Uint8Array([{}])",
            bytes
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Dict(members) => format!(
            "{{{}}}",
            members
                .iter()
                .map(|(name, v)| Some(format!("{}: {}", text(name), js(v)?)))
                .collect::<Option<Vec<_>>>()?
                .join(", ")
        ),
        Value::List(items) => format!(
            "[{}]",
            items.iter().map(js).collect::<Option<Vec<_>>>()?.join(", ")
        ),
        Value::Key(_) | Value::Other => return None,
    })
}

/// The bindings' lists (the bootstrap's JsonWebKey members, usages,
/// formats and methods) are the store's, member for member and in order
/// (the lead's ruling, G255 5c): a member the bindings drop never
/// reaches the store, which the store's own tests cannot see.
#[test]
fn the_bindings_lists_are_the_stores() {
    let source = include_str!("../../../js/src/quickjs/bootstrap.js");
    let list = |name: &str| -> Vec<String> {
        let head = format!("const {name} = [");
        assert_eq!(source.matches(&head).count(), 1, "{name}");
        let rest = &source[source.find(&head).expect(name) + head.len()..];
        let body = &rest[..rest.find("];").expect("its end")];
        body.split('"')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect()
    };
    assert_eq!(list("JWK_MEMBERS"), crypto::JWK_MEMBERS);
    assert_eq!(
        list("KEY_USAGES"),
        crypto::Usage::ALL.map(crypto::Usage::name)
    );
    assert_eq!(list("KEY_FORMATS"), crypto::FORMATS);
    assert_eq!(list("SUBTLE_METHODS"), crypto::METHODS);
}

/// SubtleCrypto.supports through a page's bindings answers as the store
/// does, for a sample of the store's own generated combinations (every
/// fourth without a key in it): a dropped argument, member or
/// conversion shows as a difference.
#[test]
fn the_pages_supports_answers_as_the_stores() {
    use crypto::Value;
    let oracle = crypto::oracle::Oracle::new();
    let keys = crypto::Keys::default();
    let sample: Vec<(&str, Value, Value, String)> = oracle
        .variants()
        .into_iter()
        .filter_map(|(op, algorithm, extra)| {
            let call = match &extra {
                Value::Absent => format!("S.supports({op:?}, {})", js(&algorithm)?),
                _ => format!("S.supports({op:?}, {}, {})", js(&algorithm)?, js(&extra)?),
            };
            Some((op, algorithm, extra, call))
        })
        .step_by(4)
        .collect();
    assert!(sample.len() >= 300, "{} combinations", sample.len());
    let calls: Vec<&str> = sample.iter().map(|(_, _, _, call)| call.as_str()).collect();
    let script = format!(
        "const S = SubtleCrypto; console.log(JSON.stringify([{}]));",
        calls.join(",\n")
    );
    let lines = logged(&script, 1);
    let answers = devtools::Json::parse(&lines[0]).expect("the page's answers");
    let answers = answers.as_array().expect("a list");
    assert_eq!(answers.len(), sample.len());
    let wrong: Vec<String> = sample
        .iter()
        .zip(answers)
        .map(|((op, algorithm, extra, call), page)| {
            (call, page, crypto::supports(&keys, op, algorithm, extra))
        })
        .filter(|(_, page, store)| page.as_bool() != Some(*store))
        .map(|(call, _, store)| format!("{call}: the store says {store}"))
        .collect();
    assert!(wrong.is_empty(), "the page differs:\n{}", wrong.join("\n"));
}

/// Every key the store makes, exported in each format it has, imported
/// through a page's bindings with the store's algorithm and usages, is
/// taken or refused as the store takes or refuses it, in its words.
#[test]
fn the_pages_imports_answer_as_the_stores() {
    let mut oracle = crypto::oracle::Oracle::new();
    let samples = oracle.import_samples();
    let cases: Vec<String> = samples
        .iter()
        .map(|(format, data, algorithm, usages, _)| {
            let usages: Vec<String> = usages.iter().map(|u| format!("{:?}", u.name())).collect();
            format!(
                "[{format:?}, {}, {}, [{}]]",
                js(data).expect("data"),
                js(algorithm).expect("an algorithm"),
                usages.join(", ")
            )
        })
        .collect();
    let script = format!(
        "(async () => {{ const cases = [{}]; const out = [];\
         for (const [f, d, a, u] of cases) {{\
           try {{ await crypto.subtle.importKey(f, d, a, true, u); out.push('ok'); }}\
           catch (e) {{ out.push(e.name + ': ' + e.message); }}\
         }}\
         console.log(JSON.stringify(out)); }})();",
        cases.join(",\n")
    );
    let lines = logged(&script, 1);
    let answers = devtools::Json::parse(&lines[0]).expect("the page's answers");
    let answers = answers.as_array().expect("a list");
    assert_eq!(answers.len(), samples.len());
    assert!(samples.len() >= 100, "{} imports", samples.len());
    let wrong: Vec<String> = samples
        .iter()
        .zip(answers)
        .filter(|((_, _, _, _, store), page)| page.as_str() != Some(store.as_str()))
        .map(|((format, _, algorithm, _, store), page)| {
            format!("{format} {algorithm:?}: page {page:?}, store {store}")
        })
        .collect();
    assert!(wrong.is_empty(), "the page differs:\n{}", wrong.join("\n"));
}

/// ML-KEM's methods and getPublicKey through a page (probed: mlkem):
/// their lengths and order on the prototype, an encapsulation's shape,
/// its decapsulation, a shared key, and a public key from a private one.
#[test]
fn a_page_encapsulates_and_decapsulates_with_ml_kem() {
    let script = "(async () => {\
        const s = crypto.subtle; const out = [];\
        const names = Object.getOwnPropertyNames(SubtleCrypto.prototype);\
        out.push(names.slice(names.indexOf('wrapKey')).join());\
        out.push(['encapsulateKey', 'encapsulateBits', 'decapsulateKey', 'decapsulateBits', 'getPublicKey'].map((m) => s[m].length).join());\
        const pair = await s.generateKey('ML-KEM-768', true, ['encapsulateKey', 'encapsulateBits', 'decapsulateKey', 'decapsulateBits']);\
        const enc = await s.encapsulateBits('ML-KEM-768', pair.publicKey);\
        out.push([Object.keys(enc).join('/'), enc.ciphertext.byteLength, enc.sharedKey.byteLength].join());\
        const dec = new Uint8Array(await s.decapsulateBits('ML-KEM-768', pair.privateKey, enc.ciphertext));\
        out.push(String(dec.every((b, i) => b === new Uint8Array(enc.sharedKey)[i])));\
        const k = await s.encapsulateKey('ML-KEM-768', pair.publicKey, {name: 'AES-GCM', length: 128}, true, ['encrypt']);\
        out.push([k.ciphertext.byteLength, k.sharedKey.type, k.sharedKey.algorithm.length, k.sharedKey.usages.join()].join());\
        const back = await s.decapsulateKey('ML-KEM-768', pair.privateKey, k.ciphertext, 'AES-GCM', true, ['decrypt']);\
        out.push(String(new Uint8Array(await s.exportKey('raw', back)).join() === new Uint8Array(await s.exportKey('raw', k.sharedKey)).join()));\
        try { await s.decapsulateBits('ML-KEM-768', pair.privateKey, new Uint8Array(5)); } catch (e) { out.push(e.name + ':' + e.message); }\
        const pub = await s.getPublicKey(pair.privateKey, ['encapsulateBits']);\
        out.push([pub.type, pub.algorithm.name, pub.usages.join(), pub.extractable].join());\
        try { await s.getPublicKey(pair.publicKey, []); } catch (e) { out.push(e.name + ': ' + e.message); }\
        out.push([SubtleCrypto.supports('encapsulateKey', 'ML-KEM-768', {name: 'AES-GCM', length: 256}), SubtleCrypto.supports('encapsulateKey', 'ML-KEM-768'), SubtleCrypto.supports('getPublicKey', 'ML-KEM-1024'), SubtleCrypto.supports('getPublicKey', 'AES-GCM')].join());\
        for (const line of out) console.log(line);\
        })().catch((e) => console.log('failed', e.name, e.message));";
    let lines = logged(script, 10);
    assert_eq!(
        lines,
        [
            "wrapKey,decapsulateBits,decapsulateKey,encapsulateBits,encapsulateKey,getPublicKey,constructor",
            "5,2,6,3,2",
            "ciphertext/sharedKey,1088,32",
            "true",
            "1088,secret,256,encrypt",
            "true",
            "DataError:",
            "public,ML-KEM-768,encapsulateBits,true",
            "InvalidAccessError: Failed to execute 'getPublicKey' on 'SubtleCrypto': getPublicKey can only be called with a private key",
            "true,false,true,false",
        ]
    );
}
