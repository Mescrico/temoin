#!/usr/bin/env python3
"""Minimal App Store Connect API helper for CI (no secrets are ever printed).

Environment: APPSTORE_ISSUER_ID, APPSTORE_KEY_ID, APPSTORE_API_KEY_P8 (the .p8 contents).

    python3 tools/appstore.py check                 # key works; team, bundle IDs, apps
    python3 tools/appstore.py ensure-bundle-ids ID… # register missing iOS bundle IDs, print TEAM_ID=…
    python3 tools/appstore.py app-exists ID         # fails with a clear message if the app record is missing
    python3 tools/appstore.py public-link ID         # TestFlight external group with a public link (created once)
    python3 tools/appstore.py metadata FILE VERSION  # App Store listing from docs/distribution/appstore.json
"""
import base64
import json
import os
import re
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


FAILURES = []


def call(method, path, body=None, soft=False):
    req = urllib.request.Request(API + path, method=method, data=json.dumps(body).encode() if body else None,
                                 headers={"Authorization": "Bearer " + token(), "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            return json.loads(r.read() or b"{}")
    except urllib.error.HTTPError as e:
        errors = json.loads(e.read() or b"{}").get("errors", [{}])
        msg = f"{method} {path}: HTTP {e.code} " + " | ".join(f"{x.get('title', '')} — {x.get('detail', '')}" for x in errors)
        if soft:
            print("  ! " + msg)
            FAILURES.append(msg)
            return None
        sys.exit("App Store Connect API " + msg)


def upsert_localizations(existing, wanted, kind, parent_type, parent_id, parent_rel):
    """PATCH the localizations that exist, POST the missing ones."""
    for locale, attrs in wanted.items():
        if locale in existing:
            r = call("PATCH", f"/{kind}/{existing[locale]['id']}",
                     {"data": {"type": kind, "id": existing[locale]["id"], "attributes": attrs}}, soft=True)
        else:
            r = call("POST", f"/{kind}", {"data": {"type": kind, "attributes": {"locale": locale, **attrs},
                                                    "relationships": {parent_rel: {"data": {"type": parent_type, "id": parent_id}}}}}, soft=True)
        print(f"  {kind} {locale}: {'ok' if r is not None else 'FAILED'}")


# Age rating answers for an app with no objectionable content: every level "NONE", every yes/no "false".
AGE_BOOLEANS = {"gambling", "unrestrictedWebAccess", "lootBox", "advertising", "ageAssurance", "healthOrWellnessTopics",
                "messagingAndChat", "parentalControls", "userGeneratedContent", "seventeenPlus"}
AGE_SKIP = {"kidsAgeBand", "ageRatingOverride", "ageRatingOverrideV2", "koreaAgeRatingOverride", "developerAgeRatingInfoUrl",
            "gracRatingClassificationNumber"}


def metadata(path, version):
    meta = json.load(open(path, encoding="utf-8"))
    app = call("GET", f"/apps?filter[bundleId]={meta['bundleId']}")["data"][0]
    app_id = app["id"]
    print("app:", app["attributes"]["name"], app_id)

    r = call("PATCH", f"/apps/{app_id}", {"data": {"type": "apps", "id": app_id,
             "attributes": {"contentRightsDeclaration": meta["contentRightsDeclaration"]}}}, soft=True)
    print("content rights:", "ok" if r is not None else "FAILED")

    infos = call("GET", f"/apps/{app_id}/appInfos")["data"]
    editable = [i for i in infos if i["attributes"].get("state", i["attributes"].get("appStoreState")) not in ("READY_FOR_DISTRIBUTION", "READY_FOR_SALE")]
    info = (editable or infos)[0]
    r = call("PATCH", f"/appInfos/{info['id']}", {"data": {"type": "appInfos", "id": info["id"], "relationships": {
        "primaryCategory": {"data": {"type": "appCategories", "id": meta["primaryCategory"]}},
        "secondaryCategory": {"data": {"type": "appCategories", "id": meta["secondaryCategory"]}}}}}, soft=True)
    print("categories:", "ok" if r is not None else "FAILED")

    locs = {l["attributes"]["locale"]: l for l in call("GET", f"/appInfos/{info['id']}/appInfoLocalizations")["data"]}
    upsert_localizations(locs, meta["info"], "appInfoLocalizations", "appInfos", info["id"], "appInfo")

    decl = call("GET", f"/appInfos/{info['id']}/ageRatingDeclaration", soft=True)
    if decl and decl.get("data"):
        attrs = {k: (False if k in AGE_BOOLEANS else "NONE") for k in decl["data"]["attributes"]
                 if k not in AGE_SKIP and not k.endswith(("Override", "Url", "Number"))}
        # Apple keeps adding questions: when it says an attribute is a BOOLEAN, answer "false" and retry.
        for _ in range(4):
            r = call("PATCH", f"/ageRatingDeclarations/{decl['data']['id']}",
                     {"data": {"type": "ageRatingDeclarations", "id": decl["data"]["id"], "attributes": attrs}}, soft=True)
            if r is not None:
                break
            fixed = re.findall(r"attribute '(\w+)'\. Expected a BOOLEAN", FAILURES[-1])
            if not fixed:
                break
            FAILURES.pop()
            for k in fixed:
                attrs[k] = False
        print("age rating:", "ok" if r is not None else "FAILED")

    versions = call("GET", f"/apps/{app_id}/appStoreVersions?filter[platform]=IOS")["data"]
    open_states = ("PREPARE_FOR_SUBMISSION", "DEVELOPER_REJECTED", "REJECTED", "METADATA_REJECTED")
    ver = next((v for v in versions if v["attributes"].get("appStoreState", v["attributes"].get("appVersionState")) in open_states), None)
    if ver is None:
        sys.exit("no editable App Store version")
    r = call("PATCH", f"/appStoreVersions/{ver['id']}", {"data": {"type": "appStoreVersions", "id": ver["id"],
             "attributes": {"versionString": version, "copyright": meta["copyright"]}}}, soft=True)
    print(f"version {version}:", "ok" if r is not None else "FAILED")

    vlocs = {l["attributes"]["locale"]: l for l in call("GET", f"/appStoreVersions/{ver['id']}/appStoreVersionLocalizations")["data"]}
    upsert_localizations(vlocs, meta["store"], "appStoreVersionLocalizations", "appStoreVersions", ver["id"], "appStoreVersion")

    builds = call("GET", f"/builds?filter[app]={app_id}&filter[preReleaseVersion.version]={version}&filter[processingState]=VALID&sort=-uploadedDate&limit=1")["data"]
    if builds:
        r = call("PATCH", f"/appStoreVersions/{ver['id']}/relationships/build", {"data": {"type": "builds", "id": builds[0]["id"]}}, soft=True)
        print(f"build {builds[0]['attributes'].get('version')}:", "ok" if r is not None else "FAILED")
    else:
        print(f"  ! no processed build {version} yet")
        FAILURES.append(f"no processed build {version}")

    review = call("GET", f"/appStoreVersions/{ver['id']}/appStoreReviewDetail", soft=True)
    if review and review.get("data"):
        # The API wants the contact (entered in App Store Connect) sent back with every change.
        attrs = {k: v for k, v in review["data"]["attributes"].items() if k.startswith("contact") and v}
        r = call("PATCH", f"/appStoreReviewDetails/{review['data']['id']}", {"data": {"type": "appStoreReviewDetails",
                 "id": review["data"]["id"], "attributes": {**attrs, "notes": meta["reviewNotes"], "demoAccountRequired": False}}}, soft=True)
    else:
        r = call("POST", "/appStoreReviewDetails", {"data": {"type": "appStoreReviewDetails",
                 "attributes": {"notes": meta["reviewNotes"], "demoAccountRequired": False},
                 "relationships": {"appStoreVersion": {"data": {"type": "appStoreVersions", "id": ver["id"]}}}}}, soft=True)
    if r is None and "contactFirstName" in FAILURES[-1]:
        FAILURES.pop()
        print("review notes: skipped (enter the review contact once in App Store Connect, then run again)")
    else:
        print("review notes:", "ok" if r is not None else "FAILED")

    # TestFlight test information (public beta): description and privacy URL per language,
    # review notes. Feedback e-mail and review contact are personal: entered in App Store Connect.
    blocs = {l["attributes"]["locale"]: l for l in call("GET", f"/apps/{app_id}/betaAppLocalizations")["data"]}
    beta = {loc: {"description": meta["store"][loc]["description"], "privacyPolicyUrl": meta["info"][loc]["privacyPolicyUrl"],
                  "marketingUrl": meta["store"][loc]["marketingUrl"]} for loc in meta["store"]}
    upsert_localizations(blocs, beta, "betaAppLocalizations", "apps", app_id, "app")
    bdetail = call("GET", f"/apps/{app_id}/betaAppReviewDetail", soft=True)
    if bdetail and bdetail.get("data"):
        battrs = {k: v for k, v in bdetail["data"]["attributes"].items() if k.startswith("contact") and v}
        r = call("PATCH", f"/betaAppReviewDetails/{bdetail['data']['id']}", {"data": {"type": "betaAppReviewDetails",
                 "id": bdetail["data"]["id"], "attributes": {**battrs, "notes": meta["reviewNotes"], "demoAccountRequired": False}}}, soft=True)
        if r is None and "contact" in FAILURES[-1]:
            FAILURES.pop()
            print("TestFlight review notes: skipped (enter the review contact once in TestFlight, then run again)")
        else:
            print("TestFlight review notes:", "ok" if r is not None else "FAILED")

    if FAILURES:
        print(f"\n{len(FAILURES)} step(s) failed:")
        for f in FAILURES:
            print(" -", f)
        sys.exit(1)
    print("\nall metadata applied")


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
    elif cmd == "internal-testing":
        # Export compliance answer chosen by the account holder for builds that don't have one yet,
        # then an internal TestFlight group with every team member, receiving every build.
        ident = sys.argv[2]
        app_id = call("GET", f"/apps?filter[bundleId]={ident}")["data"][0]["id"]
        for b in call("GET", f"/builds?filter[app]={app_id}&sort=-uploadedDate&limit=10")["data"]:
            if b["attributes"].get("usesNonExemptEncryption") is None and b["attributes"].get("processingState") == "VALID":
                r = call("PATCH", f"/builds/{b['id']}", {"data": {"type": "builds", "id": b["id"],
                         "attributes": {"usesNonExemptEncryption": False}}}, soft=True)
                print(f"build {b['attributes']['version']}: export compliance", "set" if r is not None else "FAILED")
        groups = call("GET", f"/apps/{app_id}/betaGroups?limit=50")["data"]
        group = next((g for g in groups if g["attributes"].get("isInternalGroup")), None)
        if group is None:
            group = call("POST", "/betaGroups", {"data": {"type": "betaGroups",
                     "attributes": {"name": "Team", "isInternalGroup": True, "hasAccessToAllBuilds": True},
                     "relationships": {"app": {"data": {"type": "apps", "id": app_id}}}}})["data"]
            print("created internal group:", group["attributes"]["name"])
        else:
            print("internal group:", group["attributes"]["name"])
        users = call("GET", "/users?limit=50")["data"]
        for u in users:
            a = u["attributes"]
            r = call("POST", "/betaTesters", {"data": {"type": "betaTesters",
                     "attributes": {"email": a["username"], "firstName": a.get("firstName", ""), "lastName": a.get("lastName", "")},
                     "relationships": {"betaGroups": {"data": [{"type": "betaGroups", "id": group["id"]}]}}}}, soft=True)
            if r is None and "409" in FAILURES[-1]:
                FAILURES.pop()  # already a tester: not an error
            print("team member added as internal tester" if r is not None else "team member: already a tester or not addable")
        if FAILURES:
            sys.exit(1)
    elif cmd == "screenshots":
        # docs/distribution/appstore-screenshots/*.png (1284x2778) on every language of the
        # editable version, replacing the previous set.
        import glob, hashlib
        folder = sys.argv[2]
        files = sorted(glob.glob(os.path.join(folder, "*.png")))
        if not files:
            sys.exit("no screenshots in " + folder)
        app_id = call("GET", "/apps?filter[bundleId]=io.github.temoin-app.temoin")["data"][0]["id"]
        versions = call("GET", f"/apps/{app_id}/appStoreVersions?filter[platform]=IOS")["data"]
        ver = next(v for v in versions if v["attributes"].get("appStoreState", v["attributes"].get("appVersionState"))
                   in ("PREPARE_FOR_SUBMISSION", "DEVELOPER_REJECTED", "REJECTED", "METADATA_REJECTED"))
        display = "APP_IPHONE_65"
        for loc in call("GET", f"/appStoreVersions/{ver['id']}/appStoreVersionLocalizations")["data"]:
            locale = loc["attributes"]["locale"]
            sets = call("GET", f"/appStoreVersionLocalizations/{loc['id']}/appScreenshotSets")["data"]
            sset = next((s for s in sets if s["attributes"]["screenshotDisplayType"] == display), None)
            if sset is None:
                sset = call("POST", "/appScreenshotSets", {"data": {"type": "appScreenshotSets",
                            "attributes": {"screenshotDisplayType": display},
                            "relationships": {"appStoreVersionLocalization": {"data": {"type": "appStoreVersionLocalizations", "id": loc["id"]}}}}})["data"]
            for old in call("GET", f"/appScreenshotSets/{sset['id']}/appScreenshots")["data"]:
                call("DELETE", f"/appScreenshots/{old['id']}")
            for f in files:
                data = open(f, "rb").read()
                shot = call("POST", "/appScreenshots", {"data": {"type": "appScreenshots",
                            "attributes": {"fileName": os.path.basename(f), "fileSize": len(data)},
                            "relationships": {"appScreenshotSet": {"data": {"type": "appScreenshotSets", "id": sset["id"]}}}}})["data"]
                for op in shot["attributes"]["uploadOperations"]:
                    part = data[op["offset"]:op["offset"] + op["length"]]
                    req = urllib.request.Request(op["url"], data=part, method=op["method"],
                                                 headers={h["name"]: h["value"] for h in op.get("requestHeaders") or []})
                    urllib.request.urlopen(req, timeout=120).read()
                call("PATCH", f"/appScreenshots/{shot['id']}", {"data": {"type": "appScreenshots", "id": shot["id"],
                     "attributes": {"uploaded": True, "sourceFileChecksum": hashlib.md5(data).hexdigest()}}})
                print(f"{locale}: {os.path.basename(f)} uploaded")
    elif cmd == "submit":
        # App Review for the editable version (with the build chosen by `metadata`), and beta
        # review of that build for the public TestFlight group. Run `metadata` first.
        version = sys.argv[2]
        app_id = call("GET", "/apps?filter[bundleId]=io.github.temoin-app.temoin")["data"][0]["id"]
        builds = call("GET", f"/builds?filter[app]={app_id}&filter[preReleaseVersion.version]={version}"
                             "&filter[processingState]=VALID&sort=-uploadedDate&limit=1")["data"]
        if not builds:
            sys.exit(f"no processed build {version} yet")
        build = builds[0]
        if build["attributes"].get("usesNonExemptEncryption") is None:
            sys.exit("answer the export compliance question for this build first")
        # Public beta: the build in the external group, then beta review.
        groups = call("GET", f"/apps/{app_id}/betaGroups?limit=50")["data"]
        public = next((g for g in groups if not g["attributes"].get("isInternalGroup") and g["attributes"].get("publicLinkEnabled")), None)
        if public:
            call("POST", f"/betaGroups/{public['id']}/relationships/builds", {"data": [{"type": "builds", "id": build["id"]}]}, soft=True)
            r = call("POST", "/betaAppReviewSubmissions", {"data": {"type": "betaAppReviewSubmissions",
                     "relationships": {"build": {"data": {"type": "builds", "id": build["id"]}}}}}, soft=True)
            print("TestFlight beta review:", "submitted" if r is not None else "FAILED")
        # App Store: one review submission holding the version.
        versions = call("GET", f"/apps/{app_id}/appStoreVersions?filter[platform]=IOS")["data"]
        ver = next(v for v in versions if v["attributes"].get("versionString") == version)
        sub = call("POST", "/reviewSubmissions", {"data": {"type": "reviewSubmissions", "attributes": {"platform": "IOS"},
                   "relationships": {"app": {"data": {"type": "apps", "id": app_id}}}}}, soft=True)
        if sub is not None:
            sid = sub["data"]["id"]
            item = call("POST", "/reviewSubmissionItems", {"data": {"type": "reviewSubmissionItems",
                        "relationships": {"reviewSubmission": {"data": {"type": "reviewSubmissions", "id": sid}},
                                          "appStoreVersion": {"data": {"type": "appStoreVersions", "id": ver["id"]}}}}}, soft=True)
            if item is not None:
                r = call("PATCH", f"/reviewSubmissions/{sid}", {"data": {"type": "reviewSubmissions", "id": sid,
                         "attributes": {"submitted": True}}}, soft=True)
                print("App Review:", "submitted" if r is not None else "FAILED")
        if FAILURES:
            sys.exit(1)
    elif cmd == "cancel-review":
        # Withdraws the version from App Review (to attach a newer build), then waits until
        # it is editable again. `metadata` then picks the newest build and `submit` resubmits.
        version = sys.argv[2]
        app_id = call("GET", "/apps?filter[bundleId]=io.github.temoin-app.temoin")["data"][0]["id"]
        subs = call("GET", f"/reviewSubmissions?filter[app]={app_id}&filter[platform]=IOS&limit=20")["data"]
        active = [s for s in subs if s["attributes"].get("state") in ("WAITING_FOR_REVIEW", "IN_REVIEW", "UNRESOLVED_ISSUES", "READY_FOR_REVIEW")]
        for s in active:
            r = call("PATCH", f"/reviewSubmissions/{s['id']}", {"data": {"type": "reviewSubmissions", "id": s["id"],
                     "attributes": {"canceled": True}}}, soft=True)
            print(f"review submission {s['attributes'].get('state')}:", "canceled" if r is not None else "FAILED")
        if not active:
            print("no active review submission")
        editable = ("PREPARE_FOR_SUBMISSION", "DEVELOPER_REJECTED", "REJECTED", "METADATA_REJECTED")
        for _ in range(40):
            versions = call("GET", f"/apps/{app_id}/appStoreVersions?filter[platform]=IOS")["data"]
            ver = next(v for v in versions if v["attributes"].get("versionString") == version)
            state = ver["attributes"].get("appStoreState") or ver["attributes"].get("appVersionState")
            if state in editable:
                break
            time.sleep(15)
        print(f"version {version}: {state}")
        if state not in editable:
            FAILURES.append(f"version {version} still {state}")
        if FAILURES:
            sys.exit(1)
    elif cmd == "metadata":
        metadata(sys.argv[2], sys.argv[3])
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
