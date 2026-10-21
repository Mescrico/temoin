// Cross-check the browser code against objects sealed by the Rust `sr seal`.
// Fixtures: test/fixtures (regenerate with the commands in README.md).
import { readFileSync } from 'node:fs';
import assert from 'node:assert/strict';
import { loadIdentity, decrypt, AgeError } from '../src/age.js';
import { decodeEnvelope } from '../src/media.js';
import { encryptFile, ciphertextLen } from '../src/sharecrypt.js';

const dir = new URL('./fixtures/', import.meta.url);
const read = (name) => new Uint8Array(readFileSync(new URL(name, dir)));
const testKey = new Uint8Array(readFileSync(new URL('../../testdata/identity.txt', import.meta.url)));

const id = loadIdentity(testKey);
for (const name of ['big', 'small', 'empty']) {
  const { header, content } = decodeEnvelope(decrypt(id, read(`${name}.age`)));
  assert.deepEqual(content, read(`${name}.bin`), `${name}: content`);
  assert.equal(header.session, `vec${name}`);
  console.log(`ok  ${name}: ${content.length} bytes`);
}

// Wrong key: a clear error, not garbage.
assert.throws(() => decrypt(id, read('prot.age')), (e) => e instanceof AgeError && e.code === 'wrong_key');
console.log('ok  wrong key refused');

// Passphrase-protected identity (what the desktop app and `sr keygen --passphrase` write).
assert.throws(() => loadIdentity(read('protected-identity.age')), (e) => e.code === 'passphrase_required');
assert.throws(() => loadIdentity(read('protected-identity.age'), 'nope'), (e) => e.code === 'wrong_passphrase');
const prot = loadIdentity(read('protected-identity.age'), 'correct horse battery staple');
assert.deepEqual(decodeEnvelope(decrypt(prot, read('prot.age'))).content, read('small.bin'));
console.log('ok  passphrase-protected identity');

// Tampering is detected.
const bad = read('big.age');
bad[bad.length - 10] ^= 1;
assert.throws(() => decrypt(id, bad), (e) => e.code === 'bad_payload');
console.log('ok  altered payload refused');

// Hand-back encryption: same sizes as the Rust sharecrypt.
for (const n of [0, 5, 1 << 20, (1 << 20) + 1, 3 * (1 << 20) + 7]) {
  const ct = await encryptFile(new Uint8Array(32).fill(7), 1, new Blob([new Uint8Array(n)]));
  assert.equal(ct.size, ciphertextLen(n), `ciphertext length for ${n}`);
}
console.log('ok  share ciphertext sizes');
