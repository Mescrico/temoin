# Browser hand-back (admin page)

Builds `server/web/handback.js`, which the admin page loads when the operator clicks
**Create the link** on a hand-back request. In the operator's browser it:

1. reads the private key file (plain `AGE-SECRET-KEY-1…` or passphrase-protected age file);
2. downloads the device's objects, still encrypted, and decrypts them (age v1);
3. reassembles each recording (Android MPEG-TS segments become one MP4 with the phone's
   rotation; iPhone fMP4 and photos are concatenated as they are);
4. re-encrypts everything with a fresh key (same format as `server/src/sharecrypt.rs`),
   uploads it and shows the one-time link and its QR code.

The key never leaves the browser. Libraries: `@noble/*` and `@scure/base` (audited, MIT),
`mux.js` (Apache-2.0), `qrcode-generator` (MIT), pinned in `package-lock.json`.

```bash
npm ci
npm test        # against objects sealed by the Rust `sr seal` (test/fixtures)
npm run build   # writes server/web/handback.js (CI checks it matches the sources)
```

Fixtures were made with the test identity of `tools/testdata` (public on purpose):
`sr seal -r $(cat ../testdata/recipient.txt) --session vecbig --kind photo --container jpeg --final -o test/fixtures/big.age test/fixtures/big.bin`
(same for `small`, `empty`), and `SR_PASSPHRASE="correct horse battery staple" sr keygen --passphrase -o test/fixtures/protected-identity.age`
then `prot.age` sealed to that identity's recipient.
