# Contributing to Témoin

Thanks for helping! Bug reports, reviews, translations, documentation and code are all welcome.

- **Security issues**: see [SECURITY.md](SECURITY.md) — never in a public issue.
- **Bugs and ideas**: open an issue with the templates provided.
- **Pull requests**: keep them focused, explain the why, include tests when you change behaviour.

## Repository layout

| Folder | Build and test |
|---|---|
| `server/` (Rust) | `cd server && cargo test && cargo clippy --all-targets -- -D warnings` |
| `desktop/` (Rust + Tauri) | `cd desktop && cargo tauri dev` — put `cloudflared.exe` in `desktop/src-tauri/resources/` first |
| `android/` (Kotlin) | `cd android && ./gradlew testDebugUnitTest assembleDebug` |
| `ios/` (Swift) | `cd ios && xcodegen generate` then Xcode; core tests: `cd ios/TemoinCore && swift test` (also works in Docker `swift:6.0-jammy`) |
| `tools/` | interoperability checks: every app's encrypted output must be readable by the official `age` tool |

The protocol is specified in [docs/PROTOCOL.md](docs/PROTOCOL.md). Changes to it must keep the three implementations (Kotlin, Swift, Rust) in sync and pass the interoperability checks in CI.

## Testing against a server

Never test against a production server: everything accepted stays in its write-only ledger. Run a local one:

```bash
cd server
cargo run --bin sr -- keygen -o /tmp/test-identity.txt      # prints the public key
SR_RECIPIENT=age1… SR_ADMIN_TOKEN=test SR_MIN_FREE_BYTES=0 cargo run
```

## Style

- Comments explain *why*, not *what*.
- User-facing text exists in English and French (Android `values/` and `values-fr/`, iOS `en.lproj` / `fr.lproj`); add both when you add a string.

## About AI-assisted code

Much of Témoin was written with an AI assistant under human review. AI-assisted contributions are welcome under the same rule: you understand, test and stand behind what you submit.

## License

By contributing, you agree that your contributions are licensed under the [AGPL-3.0-or-later](LICENSE).
