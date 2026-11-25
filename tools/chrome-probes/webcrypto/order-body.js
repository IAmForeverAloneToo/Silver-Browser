async function orderProbe() {
  const s = crypto.subtle;
  const out = [];
  const now = () => performance.now();
  const key = await s.importKey('raw', new Uint8Array(16), 'PBKDF2', false, ['deriveBits']);
  const pb = (iterations) => s.deriveBits({name: 'PBKDF2', hash: 'SHA-256', salt: new Uint8Array(8), iterations}, key, 256);
  const race = async (label, first, second) => {
    const order = []; const t = now();
    const a = first().then(() => order.push('first ' + (now() - t).toFixed(0)));
    const b = second().then(() => order.push('second ' + (now() - t).toFixed(0)));
    await Promise.all([a, b]);
    out.push(label + ': ' + order.join(', '));
  };
  const hk = await s.importKey('raw', new Uint8Array(32), {name: 'HMAC', hash: 'SHA-256'}, false, ['sign']);
  await race('pbkdf2 then importKey', () => pb(2000000), () => s.importKey('raw', new Uint8Array(16), 'AES-GCM', false, ['encrypt']));
  await race('pbkdf2 then hmac sign', () => pb(2000000), () => s.sign('HMAC', hk, new Uint8Array(16)));
  await race('pbkdf2 then aes generateKey', () => pb(2000000), () => s.generateKey({name: 'AES-GCM', length: 256}, false, ['encrypt']));
  await race('pbkdf2 then ec generateKey', () => pb(2000000), () => s.generateKey({name: 'ECDSA', namedCurve: 'P-256'}, false, ['sign']));
  await race('pbkdf2 then short pbkdf2', () => pb(2000000), () => pb(1));
  await race('rsa2048 then short pbkdf2', () => s.generateKey({name: 'RSA-PSS', modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: 'SHA-256'}, false, ['sign']), () => pb(1));
  const big = new Uint8Array(128 << 20);
  await race('digest 128 MiB then digest 16 B', () => s.digest('SHA-256', big), () => s.digest('SHA-256', new Uint8Array(16)));
  await race('digest 128 MiB then pbkdf2 1', () => s.digest('SHA-256', big), () => pb(1));
  let t = now();
  await s.generateKey({name: 'RSA-PSS', modulusLength: 8192, publicExponent: new Uint8Array([1, 0, 1]), hash: 'SHA-256'}, false, ['sign']);
  out.push('rsa8192 alone ms ' + (now() - t).toFixed(0));
  await race('rsa8192 then digest', () => s.generateKey({name: 'RSA-PSS', modulusLength: 8192, publicExponent: new Uint8Array([1, 0, 1]), hash: 'SHA-256'}, false, ['sign']), () => s.digest('SHA-256', new Uint8Array(16)));
  return out;
}
