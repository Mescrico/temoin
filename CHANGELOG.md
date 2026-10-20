# Changelog

## v0.1.0 — first public release

First open-source release of Témoin.

- Android and iOS apps: photo, video and live capture, encrypted in memory (age), uploaded in 2–4 s segments while recording; per-phone anonymous Ed25519 identities; invitations by QR code; quick recording from the lock screen; "Get my files back" request codes.
- Server (Rust, Docker image or Linux service): write-only storage, hash-chained ledger, quotas, discreet mode, admin page, journal, one-time hand-back links, Cloudflare quick tunnel or built-in Let's Encrypt HTTPS.
- `sr` CLI: key generation (passphrase-protected), verify, incremental decrypt with MP4 remux, hand-back.
- Windows desktop server: setup wizard, start/stop with Internet access, invitations, recovery, hand-back, full admin page.
