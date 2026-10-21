// Hand-back link encryption, same format as server/src/sharecrypt.rs (decrypted by the
// /r/<id> page): AES-256-GCM, 1 MiB chunks, nonce = BE32(file) ‖ BE32(chunk) ‖ BE32(last).
export const CHUNK = 1 << 20;
export const TAG = 16;

export const chunkCount = (n) => Math.max(1, Math.ceil(n / CHUNK));
export const ciphertextLen = (n) => n + TAG * chunkCount(n);

export function nonce(file, chunk, last) {
  const n = new Uint8Array(12);
  const v = new DataView(n.buffer);
  v.setUint32(0, file);
  v.setUint32(4, chunk);
  v.setUint32(8, last ? 1 : 0);
  return n;
}

export const newKey = () => crypto.getRandomValues(new Uint8Array(32));

/** The URL fragment carrying the key (base64url without padding, 43 characters). */
export function keyFragment(key) {
  let s = '';
  for (const b of key) s += String.fromCharCode(b);
  return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

function concat(parts) {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let o = 0;
  for (const p of parts) { out.set(p, o); o += p.length; }
  return out;
}

/** Encrypts file number `file` (a Blob) and yields upload parts of up to `partChunks` chunks. */
export async function* encryptParts(key, file, blob, partChunks = 8) {
  const k = await crypto.subtle.importKey('raw', key, 'AES-GCM', false, ['encrypt']);
  const n = chunkCount(blob.size);
  let part = [];
  for (let i = 0; i < n; i++) {
    const plain = new Uint8Array(await blob.slice(i * CHUNK, Math.min(blob.size, (i + 1) * CHUNK)).arrayBuffer());
    part.push(new Uint8Array(await crypto.subtle.encrypt({ name: 'AES-GCM', iv: nonce(file, i, i === n - 1) }, k, plain)));
    if (part.length === partChunks || i === n - 1) {
      yield concat(part);
      part = [];
    }
  }
}

/** Whole ciphertext of one file (tests). */
export async function encryptFile(key, file, blob) {
  const parts = [];
  for await (const p of encryptParts(key, file, blob)) parts.push(p);
  return new Blob(parts);
}
