async function lane(iterations) {
  const s = crypto.subtle;
  const key = await s.importKey('raw', new Uint8Array(16), 'PBKDF2', false, ['deriveBits']);
  const t = performance.now();
  await s.deriveBits({name: 'PBKDF2', hash: 'SHA-256', salt: new Uint8Array(8), iterations}, key, 256);
  return performance.now() - t;
}
