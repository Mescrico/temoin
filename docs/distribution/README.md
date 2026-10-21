# Distributing the apps

Where each build comes from and what is needed to publish it. Everything is built by GitHub Actions on version tags (`vX.Y.Z`).

| Channel | Built by | Signed with | Status |
|---|---|---|---|
| GitHub releases — Android APK | `android.yml` | Témoin release key (repository secrets) | live |
| GitHub releases — iOS unsigned IPA | `ios.yml` | none (sideloading) | live |
| GitHub releases — Windows, macOS | `desktop.yml` | Windows: unsigned · macOS: Developer ID, notarized | live |
| TestFlight / App Store | `ios-testflight.yml` | Apple cloud signing (App Store Connect API key) | TestFlight |
| Google Play | `android.yml` → artifact `Temoin-*-play.aab` | same release key as upload key (Play App Signing) | not yet |
| F-Droid | F-Droid's build server, from source | F-Droid's key | to submit |
| IzzyOnDroid | the APK from the GitHub release | Témoin release key | to submit |

## F-Droid

Free and open source only, built from source by F-Droid. The app has no proprietary dependency (no Google Play services, no Firebase).

1. Fork https://gitlab.com/fdroid/fdroiddata and add [`io.github.temoin_app.temoin.yml`](io.github.temoin_app.temoin.yml) as `metadata/io.github.temoin_app.temoin.yml`.
2. Open a merge request (template "App inclusion"). Review usually takes a few weeks.

F-Droid signs with its own key: an app installed from F-Droid can't be updated with the GitHub APK and the other way round.

## IzzyOnDroid

Faster than F-Droid, ships the APK from the GitHub release (same signature as the GitHub APK).

Open an inclusion request at https://gitlab.com/IzzyOnDroid/repo/-/issues with the repository URL; the listing texts come from `fastlane/metadata/android/`.

## Google Play

- Developer account: 25 USD once. A **personal** account created after November 2023 must run a closed test with at least 12 testers for 14 days before going to production; an **organization** account (needs a D-U-N-S number) skips it.
- Upload the `Temoin-*-play.aab` artifact. Enroll in Play App Signing with the existing release key as the upload key, so the GitHub APK and the Play version stay compatible.
- Forms: privacy policy ([PRIVACY.md](../PRIVACY.md)), data safety (no data collected by the developer; data is sent encrypted to a server chosen by the user), content rating, foreground service declaration (camera, microphone: recording continues with the screen off), location (optional, foreground only).

## App Store

- Build: `ios-testflight.yml` uploads to App Store Connect (run it from the Actions tab or push a tag).
- Encryption: the app uses standard algorithms (X25519, ChaCha20-Poly1305) not provided by iOS. Answer the export compliance questions accordingly; distribution in **France** requires a declaration to the ANSSI to be uploaded in App Store Connect.
- App Review needs a working server: give the reviewer an invitation link to a demo server (not a production one) in the review notes.
- Privacy: "Data Not Collected" (the developer collects nothing; recordings go encrypted to the server the user joins), privacy policy URL: [PRIVACY.md](../PRIVACY.md).
