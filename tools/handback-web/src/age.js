// age v1 decryption (https://age-encryption.org/v1): X25519 and scrypt recipients, header
// MAC, ChaCha20-Poly1305 STREAM payload. Only decryption: phones and the server seal.
import { x25519 } from '@noble/curves/ed25519.js';
import { chacha20poly1305 } from '@noble/ciphers/chacha.js';
import { hkdf } from '@noble/hashes/hkdf.js';
import { hmac } from '@noble/hashes/hmac.js';
import { sha256 } from '@noble/hashes/sha2.js';
import { scrypt } from '@noble/hashes/scrypt.js';
import { bech32, base64nopad } from '@scure/base';

const enc = new TextEncoder();
const dec = new TextDecoder();
const INTRO = 'age-encryption.org/v1';
const CHUNK = 64 * 1024;
const TAG = 16;

export class AgeError extends Error {
  constructor(code, message) { super(message || code); this.code = code; }
}

function equal(a, b) {
  if (a.length !== b.length) return false;
  let d = 0;
  for (let i = 0; i < a.length; i++) d |= a[i] ^ b[i];
  return d === 0;
}

function b64(s) {
  try { return base64nopad.decode(s); } catch (_) { throw new AgeError('bad_header', 'invalid base64 in age header'); }
}

/** Parses the text header; returns stanzas, the bytes covered by the MAC, the MAC and the payload offset. */
export function parseHeader(bytes) {
  // The header is ASCII and ends with the line "--- <mac>\n".
  const max = Math.min(bytes.length, 64 * 1024);
  let end = -1;
  for (let i = 0; i + 4 < max; i++) {
    if (bytes[i] === 0x0a && bytes[i + 1] === 0x2d && bytes[i + 2] === 0x2d && bytes[i + 3] === 0x2d && bytes[i + 4] === 0x20) { end = i + 1; break; }
  }
  if (end < 0) throw new AgeError('bad_header', 'not an age file');
  const nl = bytes.indexOf(0x0a, end);
  if (nl < 0) throw new AgeError('bad_header', 'truncated age header');
  const lines = dec.decode(bytes.subarray(0, end - 1)).split('\n');
  if (lines[0] !== INTRO) throw new AgeError('bad_header', 'unsupported age version');
  const stanzas = [];
  let cur = null;
  for (const line of lines.slice(1)) {
    if (line.startsWith('-> ')) {
      cur = { args: line.slice(3).split(' '), body: [], done: false };
      stanzas.push(cur);
    } else if (cur && !cur.done) {
      cur.body.push(line);
      if (line.length < 64) cur.done = true;
    } else {
      throw new AgeError('bad_header', 'malformed age header');
    }
  }
  for (const s of stanzas) s.body = b64(s.body.join(''));
  const macLine = dec.decode(bytes.subarray(end, nl));
  return { stanzas, macInput: bytes.subarray(0, end + 3), mac: b64(macLine.slice(4)), payload: nl + 1 };
}

function unwrap(key, body) {
  try { return chacha20poly1305(key, new Uint8Array(12)).decrypt(body); } catch (_) { return null; }
}

function payload(fileKey, bytes, offset) {
  const nonce = bytes.subarray(offset, offset + 16);
  if (nonce.length !== 16) throw new AgeError('bad_payload', 'truncated age payload');
  const key = hkdf(sha256, fileKey, nonce, enc.encode('payload'), 32);
  const ct = bytes.subarray(offset + 16);
  const size = CHUNK + TAG;
  const count = Math.max(1, Math.ceil(ct.length / size));
  const out = new Uint8Array(ct.length - count * TAG);
  let w = 0;
  for (let i = 0; i < count; i++) {
    const chunk = ct.subarray(i * size, Math.min(ct.length, (i + 1) * size));
    const n = new Uint8Array(12);
    let c = i;
    for (let j = 10; j >= 0 && c > 0; j--) { n[j] = c & 0xff; c = Math.floor(c / 256); }
    n[11] = i === count - 1 ? 1 : 0;
    let pt;
    try { pt = chacha20poly1305(key, n).decrypt(chunk); } catch (_) { throw new AgeError('bad_payload', 'age payload altered'); }
    if (i < count - 1 && pt.length !== CHUNK) throw new AgeError('bad_payload', 'bad age chunk');
    if (i === count - 1 && pt.length === 0 && count > 1) throw new AgeError('bad_payload', 'empty last age chunk');
    out.set(pt, w);
    w += pt.length;
  }
  return out;
}

function checkMac(fileKey, h) {
  const key = hkdf(sha256, fileKey, new Uint8Array(0), enc.encode('header'), 32);
  if (!equal(hmac(sha256, key, h.macInput), h.mac)) throw new AgeError('bad_header', 'age header MAC mismatch');
}

/** An X25519 identity from the text of an identity file ("AGE-SECRET-KEY-1…" line). */
export function parseIdentity(text) {
  const line = text.split(/\r?\n/).map((l) => l.trim()).find((l) => l.startsWith('AGE-SECRET-KEY-1'));
  if (!line) throw new AgeError('no_identity', 'no AGE-SECRET-KEY-1 line in this file');
  const { prefix, words } = bech32.decode(line.toLowerCase(), 128);
  if (prefix !== 'age-secret-key-') throw new AgeError('no_identity', 'not an age identity');
  const secret = Uint8Array.from(bech32.fromWords(words));
  if (secret.length !== 32) throw new AgeError('no_identity', 'bad identity length');
  return { secret, public: x25519.getPublicKey(secret) };
}

/** True if this identity file is itself age-encrypted with a passphrase. */
export function isProtected(bytes) {
  return dec.decode(bytes.subarray(0, INTRO.length)) === INTRO;
}

/** Reads an identity file (plain text, or age-encrypted with a passphrase). */
export function loadIdentity(bytes, passphrase) {
  if (!isProtected(bytes)) return parseIdentity(dec.decode(bytes));
  if (!passphrase) throw new AgeError('passphrase_required', 'this key file is protected by a passphrase');
  const h = parseHeader(bytes);
  const s = h.stanzas.find((x) => x.args[0] === 'scrypt');
  if (!s || h.stanzas.length !== 1) throw new AgeError('bad_header', 'expected a passphrase-protected file');
  const salt = b64(s.args[1]);
  const logN = Number(s.args[2]);
  if (!(logN >= 1 && logN <= 22)) throw new AgeError('bad_header', 'scrypt work factor out of range');
  const label = enc.encode('age-encryption.org/v1/scrypt');
  const fullSalt = new Uint8Array(label.length + salt.length);
  fullSalt.set(label); fullSalt.set(salt, label.length);
  const key = scrypt(enc.encode(passphrase), fullSalt, { N: 2 ** logN, r: 8, p: 1, dkLen: 32, maxmem: 2 ** 31 });
  const fileKey = unwrap(key, s.body);
  if (!fileKey) throw new AgeError('wrong_passphrase', 'wrong passphrase');
  checkMac(fileKey, h);
  return parseIdentity(dec.decode(payload(fileKey, bytes, h.payload)));
}

/** Decrypts an age file sealed to an X25519 recipient. */
export function decrypt(identity, bytes) {
  const h = parseHeader(bytes);
  for (const s of h.stanzas) {
    if (s.args[0] !== 'X25519' || s.args.length !== 2) continue;
    const share = b64(s.args[1]);
    if (share.length !== 32) continue;
    let shared;
    try { shared = x25519.getSharedSecret(identity.secret, share); } catch (_) { continue; }
    if (shared.every((b) => b === 0)) continue;
    const salt = new Uint8Array(64);
    salt.set(share); salt.set(identity.public, 32);
    const key = hkdf(sha256, shared, salt, enc.encode('age-encryption.org/v1/X25519'), 32);
    const fileKey = unwrap(key, s.body);
    if (!fileKey) continue;
    checkMac(fileKey, h);
    return payload(fileKey, bytes, h.payload);
  }
  throw new AgeError('wrong_key', 'this key does not open these recordings');
}
