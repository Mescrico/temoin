// Hand-back from the admin page. The operator's key file is read by the browser and never
// sent anywhere: the server only serves the encrypted objects and receives files
// re-encrypted with a fresh key that lives in the link fragment.
import qrcode from 'qrcode-generator';
import { loadIdentity, isProtected, AgeError } from './age.js';
import * as age from './age.js';
import { decodeEnvelope, assemble } from './media.js';
import { ciphertextLen, encryptParts, keyFragment, newKey } from './sharecrypt.js';

export { isProtected, AgeError };

function client(token, base = '') {
  const auth = { Authorization: 'Bearer ' + token };
  const check = async (r) => {
    if (!r.ok) throw new Error((await r.json().catch(() => ({}))).message || `${r.status} ${r.statusText}`);
    return r;
  };
  return {
    json: async (path, method = 'GET', body) => (await check(await fetch(base + path, {
      method, headers: { ...auth, 'Content-Type': 'application/json' }, body: body === undefined ? undefined : JSON.stringify(body),
    }))).json(),
    bytes: async (path) => new Uint8Array(await (await check(await fetch(base + path, { headers: auth, cache: 'no-store' }))).arrayBuffer()),
    put: async (path, data) => (await check(await fetch(base + path, {
      method: 'PUT', headers: { ...auth, 'Content-Type': 'application/octet-stream' }, body: data,
    }))).json(),
  };
}

/**
 * Creates the one-time link for a request code.
 *   keyFile: Uint8Array of the identity file; passphrase: if it is protected.
 *   onProgress({ step: 'download' | 'decrypt' | 'video' | 'upload', done, total })
 * Returns { link, expires_at, files: [{ name, size, complete }] }.
 */
export async function handBack({ code, keyFile, passphrase, days = 7, token, base = '', onProgress = () => {} }) {
  const api = client(token, base);
  const identity = loadIdentity(keyFile, passphrase);
  try {
    const req = await api.json(`/api/v1/admin/requests/${encodeURIComponent(code)}/objects`);
    if (req.cancelled) throw new Error('this request was cancelled');
    if (!req.objects.length) throw new Error('nothing to hand back for this device');

    // 1. Download and decrypt, one object at a time.
    const parts = [];
    const total = req.objects.length;
    for (const [i, o] of req.objects.entries()) {
      onProgress({ step: 'decrypt', done: i, total });
      const bytes = await api.bytes(`/api/v1/admin/objects/${o.session}/${o.seq}`);
      const { header, content } = decodeEnvelope(age.decrypt(identity, bytes));
      parts.push({ session: o.session, seq: o.seq, header, content });
    }
    onProgress({ step: 'video', done: 0, total: 1 });
    const media = assemble(parts);
    parts.length = 0;

    // 2. Fresh key, manifest, one share sized in advance, upload in parts, seal.
    const manifest = new Blob([JSON.stringify({
      v: 1,
      files: media.map((m) => ({ name: m.name, type: m.type, size: m.blob.size, kind: m.kind, created_at: m.created_at, complete: m.complete })),
    })]);
    const files = [manifest, ...media.map((m) => m.blob)];
    const sizes = files.map((f) => ciphertextLen(f.size));
    const all = sizes.reduce((a, b) => a + b, 0);
    const key = newKey();
    const created = await api.json('/api/v1/admin/shares', 'POST', { code: req.code, files: sizes, days, label: req.code, upto_index: req.upto_index });
    let sent = 0;
    for (const [n, f] of files.entries()) {
      let offset = 0;
      for await (const part of encryptParts(key, n, f)) {
        await api.put(`/api/v1/admin/shares/${created.id}/files/${n}?offset=${offset}`, part);
        offset += part.length;
        sent += part.length;
        onProgress({ step: 'upload', done: sent, total: all });
      }
    }
    await api.json(`/api/v1/admin/shares/${created.id}/seal`, 'POST');
    const link = `${created.link}#${keyFragment(key)}`;
    key.fill(0);
    return { link, expires_at: created.expires_at, files: media.map((m) => ({ name: m.name, size: m.blob.size, complete: m.complete })) };
  } finally {
    identity.secret.fill(0);
  }
}

/** QR code of the link, drawn in the browser (the key in the link never reaches the server). */
export function qrSvg(text) {
  const q = qrcode(0, 'L');
  q.addData(text);
  q.make();
  return q.createSvgTag({ cellSize: 4, margin: 4, scalable: true });
}
