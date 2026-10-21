#!/usr/bin/env python3
"""Minimal App Store Connect API helper for CI (no secrets are ever printed).

Environment: APPSTORE_ISSUER_ID, APPSTORE_KEY_ID, APPSTORE_API_KEY_P8 (the .p8 contents).

    python3 tools/appstore.py check                 # key works; team, bundle IDs, apps
    python3 tools/appstore.py ensure-bundle-ids ID… # register missing iOS bundle IDs, print TEAM_ID=…
    python3 tools/appstore.py app-exists ID         # fails with a clear message if the app record is missing
    python3 tools/appstore.py public-link ID         # TestFlight external group with a public link (created once)
"""
import base64
import json
import os
import sys
import time
import urllib.error
import urllib.request

from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives.asymmetric.utils import decode_dss_signature

API = "https://api.appstoreconnect.apple.com/v1"


def b64url(b):
    return base64.urlsafe_b64encode(b).rstrip(b"=").decode()


def token():
    pem = os.environ["APPSTORE_API_KEY_P8"].strip().encode()
    if b"BEGIN PRIVATE KEY" not in pem:
        sys.exit("APPSTORE_API_KEY_P8 is not a .p8 private key (expected '-----BEGIN PRIVATE KEY-----').")
    key = serialization.load_pem_private_key(pem, password=None)
    if not isinstance(key, ec.EllipticCurvePrivateKey):
        sys.exit("APPSTORE_API_KEY_P8 is not an EC key.")
    header = {"alg": "ES256", "kid": os.environ["APPSTORE_KEY_ID"].strip(), "typ": "JWT"}
    now = int(time.time())
    payload = {"iss": os.environ["APPSTORE_ISSUER_ID"].strip(), "iat": now, "exp": now + 900, "aud": "appstoreconnect-v1"}
    signing_input = f"{b64url(json.dumps(header).encode())}.{b64url(json.dumps(payload).encode())}".encode()
    r, s = decode_dss_signature(key.sign(signing_input, ec.ECDSA(hashes.SHA256())))
    return signing_input.decode() + "." + b64url(r.to_bytes(32, "big") + s.to_bytes(32, "big"))


def call(method, path, body=None):
    req = urllib.request.Request(API + path, method=method, data=json.dumps(body).encode() if body else None,
                                 headers={"Authorization": "Bearer " + token(), "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            return json.loads(r.read() or b"{}")
    except urllib.error.HTTPError as e:
        detail = json.loads(e.read() or b"{}").get("errors", [{}])[0]
        sys.exit(f"App Store Connect API {method} {path}: HTTP {e.code} {detail.get('title', '')} — {detail.get('detail', '')}")


def bundle_ids():
    return {b["attributes"]["identifier"]: b for b in call("GET", "/bundleIds?limit=200")["data"]}


def team_id(bids):
    for b in bids.values():
        if b["attributes"].get("seedId"):
            return b["attributes"]["seedId"]
    return None


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else "check"
    if cmd == "check":
        bids = bundle_ids()
        apps = call("GET", "/apps?limit=200")["data"]
        print("API key OK")
        print("team:", team_id(bids) or "(unknown until a bundle ID exists)")
        print("bundle IDs:", ", ".join(sorted(bids)) or "none")
        print("apps:", ", ".join(f"{a['attributes']['name']} ({a['attributes']['bundleId']})" for a in apps) or "none")
    elif cmd == "ensure-bundle-ids":
        bids = bundle_ids()
        for ident in sys.argv[2:]:
            if ident not in bids:
                name = "Temoin " + ident.rsplit(".", 1)[-1]
                call("POST", "/bundleIds", {"data": {"type": "bundleIds", "attributes": {"identifier": ident, "name": name, "platform": "IOS"}}})
                print("registered", ident)
            else:
                print("exists", ident)
        team = team_id(bundle_ids())
        if not team:
            sys.exit("could not find the team ID")
        print(f"TEAM_ID={team}")
        if os.environ.get("GITHUB_OUTPUT"):
            with open(os.environ["GITHUB_OUTPUT"], "a") as f:
                f.write(f"team={team}\n")
    elif cmd == "app-exists":
        ident = sys.argv[2]
        apps = call("GET", f"/apps?filter[bundleId]={ident}")["data"]
        if not apps:
            sys.exit(f"No App Store Connect app for {ident} yet. Create it once at "
                     "https://appstoreconnect.apple.com → Apps → + → New App "
                     f"(platform iOS, bundle ID {ident}), then run this workflow again.")
        print("app:", apps[0]["attributes"]["name"], apps[0]["id"])
    elif cmd == "public-link":
        ident = sys.argv[2]
        apps = call("GET", f"/apps?filter[bundleId]={ident}")["data"]
        if not apps:
            sys.exit(f"No App Store Connect app for {ident}.")
        app_id = apps[0]["id"]
        groups = call("GET", f"/apps/{app_id}/betaGroups?limit=50")["data"]
        group = next((g for g in groups if not g["attributes"].get("isInternalGroup") and g["attributes"].get("publicLinkEnabled")), None)
        if group is None:
            group = call("POST", "/betaGroups", {"data": {
                "type": "betaGroups",
                "attributes": {"name": "Public beta", "publicLinkEnabled": True, "publicLinkLimitEnabled": False},
                "relationships": {"app": {"data": {"type": "apps", "id": app_id}}},
            }})["data"]
            print("created group:", group["attributes"]["name"])
        print("public link:", group["attributes"].get("publicLink"))
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
