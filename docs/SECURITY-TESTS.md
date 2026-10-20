# Security test plan

Run against a **local, disposable**
server — never production: accepted objects stay in the write-only ledger forever).

Attacker models:

* **A — Internet scanner**: knows nothing, probes the URL.
* **B — Invitation holder**: has a QR / code, may be malicious (troll, infiltrator).
* **C — Hostile network**: can see/alter traffic before TLS (captive portal, fake Wi-Fi).
* **D — Hostile server** (for the apps): a fake server the phone is lured to.
* **E — Malicious object**: a valid encrypted object crafted by B, later processed
  by the operator's offline machine (`sr decrypt`).

## 1. Discovery (A)

| # | Test | Expected |
|---|---|---|
| 1.1 | `/`, `/api/v1/info`, admin routes, random paths, `robots.txt` without credentials (stealth) | bare `404`, empty body |
| 1.2 | `/admin` (stealth) | neutral login page: no server or product name |
| 1.3 | `X-Robots-Tag` on every response | `noindex, nofollow` |
| 1.4 | No `Server` / version header | none |
| 1.5 | Unknown methods (`OPTIONS`, `TRACE`, `DELETE`, `PATCH`) on every route | `404/405`, never content |
| 1.6 | `GET` on an object path | `405`, no read route exists |

## 2. Authentication (A, B)

| # | Test | Expected |
|---|---|---|
| 2.1 | Upload without headers / with garbage headers | `404` (stealth) or `401` |
| 2.2 | Signature by an unregistered key | `401 unknown_device` |
| 2.3 | Valid signature replayed on another object path | `401` |
| 2.4 | Valid signature, different body (hash mismatch) | `400` |
| 2.5 | Timestamp ±2 days, non-numeric, negative, huge | `401` |
| 2.6 | Device id of victim + attacker signature | `401` |
| 2.7 | Malformed / empty / oversized base64 signature | `401` |
| 2.8 | Banned device: upload, `/me`, re-register | `403` |
| 2.9 | Register with all-zero / small-order / malformed public key | `400` |
| 2.10 | Register with signature over another invite or another key | `400` |
| 2.11 | Unknown / revoked / expired / exhausted invite | `404` / `410` |
| 2.12 | Invite code with injection payloads (`' OR 1=1--`, `../`, 10 KB, NUL, unicode) | `404`, no crash |
| 2.13 | Admin API: no token, wrong token, token in query string, `Basic` scheme | `404` (stealth) / `401` |

## 3. Upload content (B)

| # | Test | Expected |
|---|---|---|
| 3.1 | Plain JPEG, MP4, PNG, PDF, ZIP, ELF, PE (`.exe`), shell script, HTML/JS, empty body, 1 byte | `400 bad_format` |
| 3.2 | age file with scrypt (passphrase) stanza | `400` |
| 3.3 | age file with 2 recipients / grease stanza / unknown stanza type | `400` |
| 3.4 | Armored (PEM) age | `400` |
| 3.5 | Non-canonical base64, wrong key length, missing MAC line, header line > 256 bytes | `400` |
| 3.6 | Truncated payload (chunk boundary) / trailing garbage making last chunk ≤ 16 bytes | `400` |
| 3.7 | Body > `max_object_bytes` (Content-Length and chunked) | `413` |
| 3.8 | Valid header + random payload of a consistent length | accepted — **documented limitation**: the server cannot read the payload by design |
| 3.9 | Quota exceeded / global cap / disk reserve | `429` / `503` |

## 4. Paths and parameters (B)

| # | Test | Expected |
|---|---|---|
| 4.1 | Session ids: `../x`, `..%2f`, `%2e%2e`, `/etc/passwd`, `C:\x`, NUL, unicode, spaces, 7 and 65 chars | `400`/`404`, **no file outside `objects/`** |
| 4.2 | seq: `-1`, `1000000`, `1e3`, `0x10`, `+1`, `01`, huge | `400` |
| 4.3 | Admin device/invite ids with traversal | `404` |
| 4.4 | Filesystem check after the whole run | only `objects/xx/<session>/<seq>.age`, `ledger.jsonl`, `devices.json`, `logs/`, `state.json` |

## 5. Stored XSS in the admin page (B → operator)

| # | Test | Expected |
|---|---|---|
| 5.1 | Invite label, device name, request path, User-Agent containing `<script>`, `<img onerror>`, `"><svg onload>` | every value escaped before `innerHTML` (review + test) |

## 6. Protocol / HTTP robustness (A, B)

| # | Test | Expected |
|---|---|---|
| 6.1 | Slow body (slowloris) | connection cut by the request timeout |
| 6.2 | Incomplete headers held open | connection cut |
| 6.3 | Content-Length shorter / longer than body | rejected, no crash |
| 6.4 | Huge / deeply nested JSON on register and admin | `4xx`, no crash |
| 6.5 | 100 parallel bad requests | server stays healthy |

## 7. Malicious objects vs `sr decrypt` (E)

| # | Test | Expected |
|---|---|---|
| 7.1 | Envelope `session` = `../../escape`, absolute path, `C:\…`, NUL | output stays inside the output directory |
| 7.2 | `kind` / `container` / `created_at` with `/`, `..`, very long | sanitized file names |
| 7.3 | Envelope length field larger than data, non-JSON header, non-UTF-8 | object reported unreadable, run continues |
| 7.4 | Two objects claiming the same session/seq | both kept, reported |

## 8. Apps vs hostile server / links (C, D)

| # | Test | Expected |
|---|---|---|
| 8.1 | Enroll link with `http://` public host | refused by platform TLS policy (Android cleartext config, iOS ATS) |
| 8.2 | Enroll link with `file:`, `javascript:`, `ftp:`, missing/garbled fields | rejected |
| 8.3 | Server returns a different key than the QR | refused before any upload |
| 8.4 | Server returns absurd limits (bitrate 10 Gbit/s, height 99999, photo 1 Gpx) | clamped by the app |
| 8.5 | Server acks with a wrong hash | object kept, retried |
