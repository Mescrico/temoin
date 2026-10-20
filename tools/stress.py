#!/usr/bin/env python3
"""Stress / crash test for a Témoin server. Never point this at production:
every accepted object stays in the write-only ledger forever.

    pip install cryptography
    python tools/stress.py --url http://127.0.0.1:18481 --admin ADMIN_TOKEN --objects DIR

DIR holds pre-sealed objects named <session>_<seq>.age (see `sr seal`).
The script creates an invitation, registers N simulated phones (Ed25519 keys),
then runs:
  1. parallel upload of every object, spread over the phones (expect 201)
  2. parallel re-upload (expect 200 duplicate)
  3. 8 simultaneous uploads of the SAME object to a fresh path (expect 1x201 + 7x200)
"""
import argparse
import base64
import concurrent.futures as cf
import hashlib
import json
import os
import sys
import time
import urllib.error
import urllib.request

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat


class Phone:
    def __init__(self):
        self.key = Ed25519PrivateKey.generate()
        self.pub = self.key.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
        self.id = hashlib.sha256(self.pub).hexdigest()[:32]

    def sign(self, msg):
        return base64.b64encode(self.key.sign(msg.encode())).decode()


def http(method, url, body=None, headers=None):
    req = urllib.request.Request(url, data=body, method=method, headers=headers or {})
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            return r.status, json.loads(r.read() or b"null")
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read() or b"null")
    except Exception as e:  # connection reset during a crash test
        return f"ERR {type(e).__name__}", None


def register(url, phone, code):
    ts = int(time.time())
    pub = base64.b64encode(phone.pub).decode()
    body = json.dumps({"invite": code, "public_key": pub, "timestamp": ts,
                       "signature": phone.sign(f"SRREG1\n{code}\n{pub}\n{ts}")}).encode()
    return http("POST", f"{url}/api/v1/devices", body, {"Content-Type": "application/json"})


def put(url, phone, session, seq, body):
    path = f"/api/v1/objects/{session}/{seq}"
    sha = hashlib.sha256(body).hexdigest()
    ts = int(time.time())
    msg = f"SRAUTH1\nPUT\n{path}\n{sha}\n{ts}\n{phone.id}"
    code, _ = http("PUT", url + path, body, {
        "X-SR-Device": phone.id, "X-SR-Timestamp": str(ts), "X-SR-Signature": phone.sign(msg),
        "X-SR-Content-SHA256": sha, "Content-Type": "application/octet-stream", "User-Agent": "sr-stress",
    })
    return code


def load(objects):
    out = []
    for name in sorted(os.listdir(objects)):
        if name.endswith(".age"):
            session, seq = name[:-4].rsplit("_", 1)
            out.append((session, int(seq), open(os.path.join(objects, name), "rb").read()))
    return out


def run(url, jobs, workers):
    t = time.time()
    with cf.ThreadPoolExecutor(workers) as ex:
        codes = list(ex.map(lambda j: put(url, *j), jobs))
    dt = max(time.time() - t, 1e-3)
    mb = sum(len(j[3]) for j in jobs) / 1e6
    hist = {}
    for c in codes:
        hist[c] = hist.get(c, 0) + 1
    print(f"  {len(jobs)} uploads, {mb:.1f} MB in {dt:.1f}s ({mb / dt:.1f} MB/s): {hist}")
    return hist


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--url", required=True)
    ap.add_argument("--admin", required=True, help="admin token")
    ap.add_argument("--objects", required=True)
    ap.add_argument("--phones", type=int, default=8)
    ap.add_argument("--workers", type=int, default=16)
    ap.add_argument("--phase", default="all", choices=["all", "upload"])
    ap.add_argument("--state", help="JSON file to reuse the same phones across runs")
    a = ap.parse_args()
    items = load(a.objects)

    phones = []
    if a.state and os.path.exists(a.state):
        for raw in json.load(open(a.state)):
            p = Phone.__new__(Phone)
            p.key = Ed25519PrivateKey.from_private_bytes(base64.b64decode(raw))
            p.pub = p.key.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
            p.id = hashlib.sha256(p.pub).hexdigest()[:32]
            phones.append(p)
    else:
        code, inv = http("POST", f"{a.url}/api/v1/admin/invites",
                         json.dumps({"label": "stress", "max_uses": a.phones}).encode(),
                         {"Authorization": f"Bearer {a.admin}", "Content-Type": "application/json"})
        assert code == 200, (code, inv)
        for _ in range(a.phones):
            p = Phone()
            code, r = register(a.url, p, inv["code"])
            assert code == 200, (code, r)
            phones.append(p)
        print(f"registered {len(phones)} phones with invitation {inv['code']}")
        if a.state:
            from cryptography.hazmat.primitives.serialization import NoEncryption, PrivateFormat
            json.dump([base64.b64encode(p.key.private_bytes(Encoding.Raw, PrivateFormat.Raw, NoEncryption())).decode()
                       for p in phones], open(a.state, "w"))

    jobs = [(phones[i % len(phones)], s, q, b) for i, (s, q, b) in enumerate(items)]
    ok = True
    print("phase 1: parallel upload")
    h = run(a.url, jobs, a.workers)
    if a.phase == "upload":
        return
    ok &= set(h) <= {201, 200}

    print("phase 2: parallel re-upload (idempotent retry)")
    h = run(a.url, jobs, a.workers)
    ok &= h == {200: len(items)}

    print("phase 3: same object x8 at once on a fresh path")
    session, _, body = items[0]
    h = run(a.url, [(phones[0], session + "_race", 0, body)] * 8, 8)
    ok &= h == {201: 1, 200: 7}

    print("RESULT:", "PASS" if ok else "FAIL")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
