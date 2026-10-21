# Threat model

What Témoin is designed to protect, against whom, and — just as important — what it does **not** protect. Read this before relying on it.

## Roles

- **Filmer**: a person with the app, joined to a server through an invitation.
- **Operator**: runs the server and holds the **private key** (ideally on a separate computer; on the desktop app, on the same PC but encrypted with a passphrase).
- **Server**: stores what phones send. Never has the private key.

## Goals

| Goal | How |
|---|---|
| Footage survives the loss of the phone | Video is cut into 2–4 s segments, each encrypted and uploaded while recording continues. A seized or broken phone loses at most the last seconds not yet uploaded (more if the network is down: the queue waits on the phone, encrypted). |
| A seized phone reveals nothing filmed | The phone only has the server's **public** key. Segments are encoded and encrypted in memory; no plaintext is written to storage. The upload queue holds ciphertext only and is excluded from backups. The app never lists what was filmed, and the server only tells a phone counters, never its file list. |
| A seized or compromised server reveals nothing filmed | The server stores age ciphertext and has no private key. Metadata (capture time, phone model, GPS, resolution) is **inside** the encryption. |
| Stored footage cannot be silently removed or altered | Hash-chained ledger of every object (`sr verify`). Nothing is ever deleted by the server, including after hand-back. |
| Filmers stay anonymous to the server | No accounts. One random Ed25519 key per phone and per server; the server knows a pseudonym. A new key for every different server, so two servers cannot link a phone. |
| Hand-back without trusting the server | The operator re-encrypts a person's files with a fresh key carried only in the link fragment (`#…`), which browsers never send to the server. One-time opening, expiry, revocation. |
| Server not discoverable | Optional **discreet mode** (`SR_STEALTH`): bare 404s to anyone without an invitation code, registered device key or admin token. |

## Attackers considered

- **Someone who takes the phone** after or during recording.
- **Someone who takes or compromises the server** (seizure, hosting provider, intruder).
- **A network observer** (Wi-Fi operator, ISP, tunnel provider).
- **A malicious invitation holder** (troll, infiltrator) uploading junk or crafted objects.
- **A hostile server** a phone could be lured to (fake QR).

Details per attacker, with test cases: [SECURITY-TESTS.md](SECURITY-TESTS.md).

## Not protected — known limits

- **While filming**, the picture is on the screen and in RAM for a few seconds. Someone forcing the unlocked phone open at that moment sees the live view.
- **The app is visible** on the phone. Its presence alone may be a risk in some places.
- **Metadata visible to the server and the network**: client IP, upload sizes and times, a random device id. Behind a Cloudflare quick tunnel, Cloudflare sees the same. Use `SR_LOG_IPS=false` to keep IPs out of the server journal; a VPN or Tor on the phone hides the IP from the server.
- **The private key is everything.** Whoever holds it (and its passphrase) can read all recordings ever sent to that server. Lost without a backup, everything is unreadable forever. On the desktop app, the key sits on the server PC, protected only by the passphrase: choose a strong one, and keep the backup copy elsewhere.
- **iOS cannot record with the screen locked or the app in the background** (system restriction); the app offers a black-screen mode instead.
- **Not evidence-grade by itself.** The ledger proves *when the server received* each object and that nothing was removed since; it does not prove *who* filmed or that the content is unaltered from capture. Signing at capture (ProofMode, C2PA) is on the roadmap.
- **No audit yet.** The age format is re-implemented in Kotlin and Swift; it is tested against the official `age` tool but has not been reviewed by an independent cryptographer.
- **Hand-back links**: whoever gets the link (with its `#…` part) gets the files. Send it through a different channel than the request code.
- **Hand-back from `/admin`**: the private key is read by the browser and never sent, but the code of that page comes from the server. A server (or a reverse proxy in front of it) that is already compromised could serve a modified page that steals the key when it is used. If that risk matters to you, give files back with the desktop app or `sr share` on a computer that does not depend on the server.
