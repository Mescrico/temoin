# Changelog

## v0.1.2

- Android: no Google dependency metadata in the APK (F-Droid / IzzyOnDroid); release builds are unsigned when no key is configured; an Android App Bundle is built for the Play Store.
- Store listings (`fastlane/metadata/android`), [privacy policy](docs/PRIVACY.md) and [distribution guide](docs/distribution/README.md).
- iOS: signed builds uploaded to TestFlight by CI.

## v0.1.1

- **Desktop server for macOS** (experimental): universal disk image for Apple Silicon and Intel Macs, with the same features as on Windows. Signed and notarized by Apple.
- Desktop texts say "this computer" instead of "this PC".

## v0.1.0 — first public release

First open-source release of Témoin.

- Android and iOS apps: photo, video and live capture, encrypted in memory (age), uploaded in 2–4 s segments while recording; per-phone anonymous Ed25519 identities; invitations by QR code; quick recording from the lock screen; "Get my files back" request codes.
- Server (Rust, Docker image or Linux service): write-only storage, hash-chained ledger, quotas, discreet mode, admin page, journal, one-time hand-back links, Cloudflare quick tunnel or built-in Let's Encrypt HTTPS.
- `sr` CLI: key generation (passphrase-protected), verify, incremental decrypt with MP4 remux, hand-back.
- Windows desktop server: guided first run, home screen with what to do next, phones (invite, rename, block), recovery, hand-back in three steps, settings; English and French, light and dark; starts with Windows and restarts the server after a reboot.
- Interfaces in English, with French when the device or browser is in French.
