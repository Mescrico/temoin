# Témoin — protocol v1

Témoin is a **write-only evidence drop**: a phone captures photos/video,
encrypts every piece to the server's public key *before it touches the disk*,
uploads it as soon as there is a network, and deletes the local copy once the
server has acknowledged it.

Nobody can read the media back through the system:

* the phone only holds the **public** key (recipient) — it can encrypt, never decrypt;
* the server exposes **no read route** at all, and by design it only needs the
  public key too. The private key (identity) lives **offline** with the operator
  and is only used on a disconnected machine with `sr decrypt`.

## 1. Keys

Standard [age](https://age-encryption.org/v1) X25519 keys.

* identity (private, offline): `AGE-SECRET-KEY-1…` — generate with `sr keygen`
  (or `age-keygen`, both are compatible).
* recipient (public, given to the server): `age1…`.

## 2. Object format

Every uploaded object is one complete **age v1 file** with exactly one
`X25519` stanza and the standard STREAM payload (ChaCha20-Poly1305, 64 KiB
chunks). Any age implementation can decrypt it: `age -d -i identity.txt x.age`.

The age plaintext is a Témoin **envelope**:

```
"SRP1"            4 bytes magic
header_len        u32 big-endian
header            header_len bytes, UTF-8 JSON (see below)
content           remaining bytes (JPEG, MPEG-TS chunk, fMP4 segment…)
```

Header JSON:

| field          | type    | meaning |
|----------------|---------|---------|
| `v`            | int     | `1` |
| `session`      | string  | random id of the capture session (one photo or one recording) |
| `seq`          | int     | 0-based index of this object inside the session |
| `kind`         | string  | `photo` \| `video` |
| `container`    | string  | `jpeg` \| `heic` \| `mpegts` \| `fmp4-init` \| `fmp4-media` |
| `final`        | bool    | `true` on the last object of the session |
| `created_at`   | string  | RFC 3339 device time when the object was produced |
| `start_ms`     | int?    | media time of the first sample relative to session start |
| `duration_ms`  | int?    | media duration of this segment |
| `device`       | object? | `{ "platform": "android", "model": "…", "app": "0.1.0", "camera": "back" }` |
| `location`     | object? | `{ "lat":…, "lon":…, "acc_m":…, "at":"…" }` — only if the user enabled it |
| `video`        | object? | `{ "codec":"avc", "width":1280, "height":720, "fps":30, "bitrate":2500000 }` |

**Every metadata field lives inside the encryption.** The server only sees:
session id, sequence number, size, upload time, IP address.

### Reassembly

Objects of one session are concatenated in `seq` order:

* `mpegts` (Android): byte concatenation of all segments is a valid MPEG-TS
  stream (continuity counters and timestamps run across segments). Each segment
  starts with PAT/PMT and an IDR frame, so any segment alone is playable too.
* `fmp4-*` (iOS): `fmp4-init` (seq 0) followed by every `fmp4-media` segment is
  a valid fragmented MP4.
* `jpeg`/`heic`: single object (`seq = 0`, `final = true`).

A missing segment leaves a gap but never breaks the others.

## 3. Devices and invitations

Every phone generates its **own Ed25519 key pair** on first launch (kept in the
app's private storage / Keychain, never backed up). The **device id** is the
first 16 bytes of `SHA-256(raw public key)`, lowercase hex (32 chars).

The operator creates **invitations** in `/admin` (label, validity, max number of
phones, per-phone quota). Each one has a QR code:

```
srec://enroll?u=<url-encoded server URL>&k=<age1 recipient>&i=<invitation code>&n=<server name>
```

The app fetches `GET /api/v1/info` and **refuses** the server if `recipient`
differs from `k` (key pinning). It then registers its public key with the
invitation code. A server typed by hand is trust-on-first-use: the app shows the
key fingerprint and asks for the invitation code. Once pinned, a changed
recipient is a hard error.

The server only stores a pseudonym ("Phone #37"), the invitation used and
usage counters — no name, e-mail or password. The operator can **ban** one
device (all its requests get `403 banned`) without affecting the others, and
**revoke** an invitation (no new phones; already registered ones keep working).

## 4. HTTP API

All bodies are JSON unless stated. Base path `/api/v1`.

### `GET /api/v1/info`

```json
{
  "server": "temoin",
  "protocol": 1,
  "version": "0.1.0",
  "name": "Riverside collective",
  "recipient": "age1…",
  "fingerprint": "52FF 8A92 7198 E413 A986",
  "accepting": true,
  "auth_required": true,
  "enrollment": "invite",
  "limits": {
    "max_object_bytes": 16777216,
    "max_video_height": 1080,
    "default_video_height": 720,
    "video_bitrate_720": 2500000,
    "video_bitrate_1080": 5000000,
    "photo_max_pixels": 4000000,
    "device_quota_bytes": 21474836480
  }
}
```

The app must never record above `max_video_height` and uses the bitrates as
upper bounds.

**Discreet servers** (`SR_STEALTH=true`) answer `/info` only to a phone sending
`X-SR-Invite: <invitation code>` or a signed request from a registered device
(even banned); everyone else gets a bare `404`, like every unknown path and every
admin route without the admin token. Apps always send one of the two.

### `POST /api/v1/devices` — register a phone

```json
{ "invite": "K7QM2…", "public_key": "<base64 32 bytes>", "timestamp": 1792412345, "signature": "<base64 64 bytes>" }
```

`signature` = Ed25519 over the UTF-8 string

```
SRREG1
<invite>
<public_key base64>
<timestamp>
```

Response `200 { "device_id", "name", "quota_bytes" }`. Registering an already
known key returns the same identity (scanning twice is harmless).
Errors: `404 unknown_invite`, `410 invite_revoked | invite_expired | invite_exhausted`,
`403 banned`, `400 bad_request`.

### Signed requests

Every other device request carries:

| header | value |
|---|---|
| `X-SR-Device` | device id |
| `X-SR-Timestamp` | unix seconds (must be within ±12 h of server time) |
| `X-SR-Signature` | base64 Ed25519 signature of the string below |
| `X-SR-Content-SHA256` | hex SHA-256 of the body (requests with a body) |

```
SRAUTH1
<METHOD>
<path>
<body sha256 hex, or "-">
<timestamp>
<device id>
```

The path is signed, so a captured signature cannot be replayed to another
object path; replaying the same upload is harmless (objects are write-once).
Errors: `401 unauthorized | unknown_device | clock_skew`, `403 banned`.

### `PUT /api/v1/objects/{session}/{seq}`

* `session`: `[A-Za-z0-9_-]{8,64}`; `seq`: `0..=999999`.
* Signed request; body = the age file (`application/octet-stream`).

The server checks the age **structure** (header grammar, one X25519 stanza,
payload length consistent with STREAM chunking) — it cannot and does not try to
decrypt. Objects are **write-once**.

| status | meaning |
|--------|---------|
| 201 | stored — receipt in body |
| 200 | identical object already stored — receipt in body (safe retry) |
| 400 | not a valid object / bad path / hash mismatch |
| 401 | missing/invalid signature, unknown device, clock skew |
| 403 | device banned |
| 409 | a *different* object already exists at this path |
| 413 | object too large |
| 429 | device quota exceeded |
| 503 | server not accepting (paused, disk reserve reached) |

Receipt:

```json
{ "sha256": "…", "size": 1234, "received_at": "2026-10-19T14:03:22Z", "index": 42, "chain": "…" }
```

The app deletes its local copy **only** after a 200/201 whose `sha256` matches.
On 401/403/429/503 it keeps the object and retries later.

### `GET /api/v1/me` — signed

`{ "device_id", "name", "quota_bytes", "used_bytes", "objects", "last_upload_at", "accepting", "request" }`
(`request`: section 5, or null).
Counters only: the server never gives a device the list of its objects, so a
seized phone reveals nothing about what was filmed.

### Admin (Bearer admin token)

* `GET /api/v1/admin/stats` — counts, bytes, free space.
* `POST /api/v1/admin/accepting` `{"accepting": false}` — pause/resume intake.
* `GET|POST /api/v1/admin/invites` — list / create `{label, expires_hours, max_uses, quota_gb}` (response includes the QR as SVG).
* `GET /api/v1/admin/invites/qr?id=…` — QR of an existing invitation.
* `POST /api/v1/admin/invites/{id}/revoke`.
* `GET /api/v1/admin/devices` — devices with usage counters.
* `POST /api/v1/admin/devices/{id}` `{"banned": true|false, "name": "…"}`.
* `GET /api/v1/admin/events?limit=N` — recent journal entries.
* `GET /api/v1/admin/requests` — hand-back requests with their device and link state (section 5).
* `GET /api/v1/admin/requests/{code}` — look a request code up (any spelling: `k7f3qx9m`).
* `POST /api/v1/admin/requests/{code}/cancel`.
* `POST /api/v1/admin/shares` `{code, files:[ciphertext sizes], days, upto_index}` → `{id, link, expires_at}`.
* `PUT /api/v1/admin/shares/{id}/files/{n}?offset=O` — upload part (≤ 9 MiB; a retried part may restart at an earlier offset, never leave a hole).
* `POST /api/v1/admin/shares/{id}/seal` · `POST /api/v1/admin/shares/{id}/revoke`.
* `GET /admin` — HTML page for all of the above.

There is deliberately **no** listing, download or delete route for objects.

## 5. Handing files back

The server cannot read anything, so returning a person's recordings goes through the
operator PC that holds the private key:

1. **Phone** — `POST /api/v1/me/request` (signed, no body) → `{code, expires_at, link}`.
   `code` is 8 characters (`K7F3-QX9M`, 40 random bits, valid 14 days), the same until its
   link is used, expires or is revoked. `GET /api/v1/me` repeats it under `request`, with
   `link` ∈ `null | uploading | ready | opened | done | expired | revoked`. The person
   passes the code to the operator by any channel: only someone holding the phone saw it.
2. **Operator PC** — `sr share --code …` looks the code up, decrypts that device's objects
   (verified ledger), remuxes videos to MP4, then encrypts every file with a **fresh random
   key** and uploads the ciphertext. Link: `https://…/r/<id>#<key>` (id = 128 random bits,
   key = 32 bytes base64url). Browsers never send the fragment: the server stores only
   ciphertext it cannot open.
3. **Recipient** — `GET /r/<id>` serves a static page (bare 404 for unknown ids) that
   decrypts in the browser with WebCrypto.

Encryption (`src/sharecrypt.rs`): AES-256-GCM over 1 MiB plaintext chunks, nonce =
`BE32(file) ‖ BE32(chunk) ‖ BE32(last ? 1 : 0)`, ciphertext chunks concatenated. File 0 is
the JSON manifest `{"v":1,"files":[{name,type,size,kind,created_at,complete}]}`, files
1..n the media in that order.

Single use: `GET /r/<id>/f/0` (manifest) is readable while the link is unopened, so the
page checks the key before anything is spent. `POST /r/<id>/open` then hands the first
caller a claim token (`X-SR-Claim`); every other caller gets `410 used`. With the token,
`GET /r/<id>/f/<n>` streams file n for one hour; `POST /r/<id>/done` (the page calls it once
the zip is downloaded) closes the link at once. Unopened links expire after `days` (1–30,
default 7). **Nothing is ever deleted**: the ciphertext of a closed link moves to
`shares-archive/<id>`. Once a link is retrieved (done, or its hour is over), the objects it
covered (that device, ledger index ≤ `upto_index` sent by `sr share`) move from `objects/`
to `returned/` (same layout), each move logged in `returned.jsonl`; they no longer count
against the device quota (`used_bytes` / `objects` in `/me` are net of them). The ledger is
untouched; `sr verify` and `sr pending` look in both folders. Content leaves the download area on done,
end of the hour, expiry or revocation; records stay in `shares.json` so the page can say why
the link no longer works.

## 6. Ledger

Each stored object appends one line to `ledger.jsonl`:

```json
{"index":42,"received_at":"…","device":"…","session":"…","seq":3,"size":1234,"sha256":"…","prev":"<chain of index 41>","chain":"sha256(prev || canonical line without chain)"}
```

The hash chain makes later removal or reordering detectable (`sr verify`).
Publishing the latest `chain` value somewhere public (or RFC 3161 timestamping
it) proves the set of objects existed at that time.
