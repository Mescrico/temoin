# Changelog

## v0.1.1

- **Desktop server for macOS** (experimental): universal disk image for Apple Silicon and Intel Macs, with the same features as on Windows. Not notarized by Apple yet; see the self-hosting guide for the first launch.
- Desktop app starts with your session and restarts the server after a reboot; launching it again shows the open window.
- iOS: capture modes show "Photo / Video / Live" instead of raw keys.
- Hand-back page: clearer message when a link is opened without https.
- Server: Docker compose and the Linux service always restart (also after a reboot).

## v0.1.0 — first public release

First open-source release of Témoin.

- Android and iOS apps: photo, video and live capture, encrypted in memory (age), uploaded in 2–4 s segments while recording; per-phone anonymous Ed25519 identities; invitations by QR code; quick recording from the lock screen; "Get my files back" request codes.
- Server (Rust, Docker image or Linux service): write-only storage, hash-chained ledger, quotas, discreet mode, admin page, journal, one-time hand-back links, Cloudflare quick tunnel or built-in Let's Encrypt HTTPS.
- `sr` CLI: key generation (passphrase-protected), verify, incremental decrypt with MP4 remux, hand-back.
- Windows desktop server: guided first run, home screen with what to do next, phones (invite, rename, block), recovery, hand-back in three steps, settings; English and French, light and dark; starts with Windows and restarts the server after a reboot.
- Interfaces in English, with French when the device or browser is in French.
