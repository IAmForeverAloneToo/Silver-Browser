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
