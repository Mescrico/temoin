# Changelog

## Unreleased

- Apps: the camera opens in video mode; the capture modes have equal widths and the chosen one sits right above the shutter.
- Android: targets API 36, as Google Play requires since 31 August 2026.
- Store listing: feature graphic.

## v0.1.5 — first public release

First open-source release of Témoin.

- Android and iOS apps: photo, video and live capture, encrypted in memory (age), uploaded in 2–4 s segments while recording; per-phone anonymous Ed25519 identities; invitations by QR code; quick recording from the lock screen; "Get my files back" request codes.
- Apps: the welcome screen shows the logo; the "Film" widget, control or shortcut used before joining a server stays on the welcome screen and explains what to do (iOS), and does not start a recording by surprise right after joining; the invitation scanner has a title, a hint and a Cancel button (iOS).
- Server (Rust, Docker image or Linux service): write-only storage, hash-chained ledger, quotas, discreet mode, admin page, journal, one-time hand-back links, Cloudflare quick tunnel or built-in Let's Encrypt HTTPS.
- **Give files back from the admin page**: *Create the link* on a request's line, choose the private key file (and its passphrase). The browser downloads the encrypted recordings, decrypts them, turns Android videos into MP4, re-encrypts everything with a fresh key and uploads it; the key file never leaves the browser. See the [threat model](docs/THREAT-MODEL.md) for the remaining risk.
- `sr` CLI: key generation (passphrase-protected), verify, incremental decrypt with MP4 remux, hand-back.
- Desktop server for Windows and macOS (universal disk image for Apple Silicon and Intel, signed and notarized by Apple): guided first run, home screen with what to do next, phones (invite, rename, block), recovery, hand-back in three steps, settings; English and French, light and dark; starts with the computer and restarts the server after a reboot.
- Distribution: Android build without Google dependency metadata (F-Droid / IzzyOnDroid) and an Android App Bundle for Google Play; iPhone app on TestFlight and submitted to the App Store (iPhone only for now); store listings (`fastlane/metadata/android`, `docs/distribution/appstore.json`), [privacy policy](docs/PRIVACY.md) and [distribution guide](docs/distribution/README.md).
- Interfaces in English, with French when the device or browser is in French.
