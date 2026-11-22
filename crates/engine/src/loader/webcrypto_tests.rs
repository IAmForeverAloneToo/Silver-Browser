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
