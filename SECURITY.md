# Security policy

Témoin is meant for people who may face real risks when filming. Security reports are the most valuable contribution you can make.

## Reporting a vulnerability

**Please do not open a public issue.** Use GitHub's private reporting instead:
**Security → Report a vulnerability** on this repository (GitHub Security Advisories).

Include what you found, how to reproduce it, and what an attacker could do with it. We aim to acknowledge reports within a week. Please give us a reasonable time to fix before public disclosure; we will credit you unless you prefer otherwise.

## Scope

In scope: the apps (`android/`, `ios/`), the server and `sr` CLI (`server/`), the desktop app (`desktop/`), the encryption formats and protocol ([docs/PROTOCOL.md](docs/PROTOCOL.md)), hand-back links.

Especially welcome:
- weaknesses in the age implementations (Kotlin `AgeSealer.kt`, Swift `AgeSealer.swift`, Rust `seal.rs` / `ageformat.rs`);
- ways for a phone, the network or the server to read content, link devices or identities, or alter the ledger undetected;
- hostile objects that harm the operator's machine during `sr decrypt`;
- ways to open a hand-back link twice or without its key.

Known limits are listed in the [threat model](docs/THREAT-MODEL.md); reports that go beyond them are welcome too.

## Status

**No independent security audit has been done yet.** Treat Témoin as alpha software.
