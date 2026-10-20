#!/usr/bin/env python3
"""Verifies device-signature vectors written by the app tests with libsodium (PyNaCl),
an implementation independent from BouncyCastle (Android) and CryptoKit (iOS).
Also checks the device id derivation: first 16 bytes of SHA-256(public key), hex."""
import base64
import hashlib
import json
import sys

from nacl.signing import VerifyKey

bad = 0
for path in sys.argv[1:]:
    v = json.load(open(path))
    pub = base64.b64decode(v["public_key"])
    try:
        VerifyKey(pub).verify(v["message"].encode(), base64.b64decode(v["signature"]))
        sig_ok = True
    except Exception:
        sig_ok = False
    id_ok = v.get("device_id", hashlib.sha256(pub).hexdigest()[:32]) == hashlib.sha256(pub).hexdigest()[:32]
    print(("OK  " if sig_ok and id_ok else "FAIL"), path)
    bad += not (sig_ok and id_ok)
if not sys.argv[1:]:
    print("no vectors"); sys.exit(1)
sys.exit(1 if bad else 0)
