//! `sr` — offline operator tool: key generation, decryption/reassembly, ledger verification.
//!
//! Run `sr decrypt` on a machine that is NOT the upload server, ideally offline:
//! it is the only place the identity (private key) should ever exist.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use age::secrecy::{ExposeSecret, SecretString};
use anyhow::{bail, Context};
use clap::{Parser, Subcommand};
use srec::envelope::{self, Header};

use srec::ops::share::{self as sr_share, Admin};
use srec::ops::*;

#[derive(Parser)]
#[command(name = "sr", version, about = "Témoin operator tool (offline)")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Generate a new identity (private key) and print its recipient (public key).
    Keygen {
        /// Write the identity to this file (refuses to overwrite). Default: stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Protect the identity file with a passphrase (asked twice, or SR_PASSPHRASE).
        #[arg(long, requires = "output")]
        passphrase: bool,
    },
    /// Print the recipient (public key) of an identity file.
    Recipient {
        #[arg(short, long)]
        identity: PathBuf,
    },
    /// Decrypt every object under DATA and reassemble photos/videos into OUT.
    Decrypt {
        #[arg(short, long)]
        identity: PathBuf,
        /// Server data directory (or its `objects/` subdirectory, or a copy of it).
        #[arg(short, long)]
        data: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        /// Also remux MPEG-TS videos to MP4 with ffmpeg (if installed).
        #[arg(long)]
        remux: bool,
        /// Skip sessions whose output already exists.
        #[arg(long)]
        skip_existing: bool,
        /// Only this device's files: its key as shown in the app ("48cc 3cdd e5bd a2c1"),
        /// at least 8 hex characters. Uses the (verified) ledger of the data directory.
        #[arg(long)]
        device: Option<String>,
        /// Only sessions that received objects at or after this ledger index (whole sessions
        /// are reassembled, including their older objects).
        #[arg(long)]
        since: Option<u64>,
        /// Incremental mode: read the starting index from this file (0 if absent) and, on
        /// success, write the index to start from next time. Lives on the operator PC only.
        #[arg(long)]
        state: Option<PathBuf>,
    },
    /// Print the objects (relative paths) to fetch for an incremental `decrypt --since/--state`:
    /// every object of the sessions touched since that index. Needs only `ledger.jsonl`.
    Pending {
        #[arg(short, long)]
        data: PathBuf,
        /// Only this device's objects (key prefix, as for `decrypt`).
        #[arg(long)]
        device: Option<String>,
        /// Only sessions that received objects at or after this ledger index (whole sessions
        /// are reassembled, including their older objects).
        #[arg(long)]
        since: Option<u64>,
        /// Incremental mode: read the starting index from this file (0 if absent) and, on
        /// success, write the index to start from next time. Lives on the operator PC only.
        #[arg(long)]
        state: Option<PathBuf>,
    },
    /// List the devices registered on the server, with their usage (from a copy of the data dir).
    Devices {
        #[arg(short, long)]
        data: PathBuf,
    },
    /// Verify the ledger hash chain and that every stored object matches its receipt.
    Verify {
        #[arg(short, long)]
        data: PathBuf,
        /// Only check the chain, do not re-hash objects.
        #[arg(long)]
        quick: bool,
        /// Only re-hash this device's objects (key prefix, as for `decrypt`).
        #[arg(long)]
        device: Option<String>,
        /// Only sessions that received objects at or after this ledger index (whole sessions
        /// are reassembled, including their older objects).
        #[arg(long)]
        since: Option<u64>,
        /// Incremental mode: read the starting index from this file (0 if absent) and, on
        /// success, write the index to start from next time. Lives on the operator PC only.
        #[arg(long)]
        state: Option<PathBuf>,
    },
    /// Look a hand-back request code up on the server and print the device id it belongs to
    /// (to copy that device's objects before `sr share`).
    Request {
        /// The code the person read in the app ("K7F3-QX9M").
        code: String,
        /// Server base URL (public URL or LAN address of the NAS).
        #[arg(long)]
        server: String,
        /// File holding the admin token (raw, or a SR_ADMIN_TOKEN=… line).
        #[arg(long)]
        token_file: PathBuf,
    },
    /// Hand a person's recordings back: decrypt the files of the device behind a request
    /// code, re-encrypt them with a fresh key, upload them and print the one-time link
    /// (https://…/r/<id>#<key>). The key is only in the link, never sent to the server.
    Share {
        #[arg(short, long)]
        identity: PathBuf,
        /// Local copy of the data directory holding that device's objects (verified ledger).
        #[arg(short, long)]
        data: PathBuf,
        /// The code the person read in the app ("K7F3-QX9M").
        #[arg(long)]
        code: String,
        #[arg(long)]
        server: String,
        #[arg(long)]
        token_file: PathBuf,
        /// Days before an unopened link expires (1-30).
        #[arg(long, default_value_t = 7)]
        days: u32,
        /// Keep the decrypted files in this folder (default: temporary folder, deleted after).
        #[arg(long)]
        keep: Option<PathBuf>,
        /// Also copy the link to the clipboard (Windows `clip`, macOS `pbcopy`).
        #[arg(long)]
        clipboard: bool,
    },
    /// Encrypt a file into a Témoin object (testing / interop helper).
    Seal {
        #[arg(short, long)]
        recipient: String,
        #[arg(long)]
        session: String,
        #[arg(long, default_value_t = 0)]
        seq: u32,
        #[arg(long, default_value = "photo")]
        kind: String,
        #[arg(long, default_value = "jpeg")]
        container: String,
        #[arg(long)]
        r#final: bool,
        input: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Print the human fingerprint of a recipient, as shown by the apps.
    Fingerprint { recipient: String },
    /// Check device-signature vectors written by the app tests (interop).
    VerifySig {
        /// JSON files: {"public_key","message","signature"} (base64 key/signature).
        files: Vec<PathBuf>,
    },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().cmd {
        Cmd::Keygen { output, passphrase } => keygen(output, passphrase),
        Cmd::Recipient { identity } => {
            println!("{}", load_identity(&identity, passphrase_for(&identity)?.as_ref())?.to_public());
            Ok(())
        }
        Cmd::Decrypt { identity, data, output, remux, skip_existing, device, since, state } => {
            let identity = load_identity(&identity, passphrase_for(&identity)?.as_ref())?;
            let mut only = device.map(|d| device_objects(&data, &d)).transpose()?;
            let window = Window::resolve(&data, since, state.as_deref())?;
            if let Some(w) = &window {
                only = Some(match only {
                    Some(dev) => dev.intersection(&w.objects).cloned().collect(),
                    None => w.objects.clone(),
                });
                if w.objects.is_empty() {
                    println!("nothing new since index {}", w.since);
                    return Ok(());
                }
            }
            decrypt(&identity, &data, &output, remux, skip_existing, only.as_ref())?.print();
            if let (Some(w), Some(state)) = (&window, &state) {
                w.save(state)?;
            }
            Ok(())
        }
        Cmd::Pending { data, device, since, state } => {
            let window = Window::resolve(&data, since, state.as_deref())?.context("give --since or --state")?;
            let dev = device.map(|d| device_objects(&data, &d)).transpose()?;
            let returned = returned_set(&data);
            let mut out = std::io::stdout().lock();
            for (session, seq) in &window.ordered {
                if dev.as_ref().is_none_or(|d| d.contains(&(session.clone(), *seq))) {
                    // Handed-back objects live in returned/ (same layout) on the server.
                    let top = if returned.contains(&(session.clone(), *seq)) { "returned" } else { "objects" };
                    writeln!(out, "{top}/{}/{session}/{seq:06}.age", &session[..2])?;
                }
            }
            eprintln!("{} objects in sessions touched since index {} (next: {})", window.objects.len(), window.since, window.next);
            Ok(())
        }
        Cmd::Devices { data } => list_devices(&data),
        Cmd::Verify { data, quick, device, since, state } => {
            let window = Window::resolve(&data, since, state.as_deref())?.map(|w| w.objects);
            let only = match (device.map(|d| device_objects(&data, &d)).transpose()?, window) {
                (Some(d), Some(w)) => Some(d.intersection(&w).cloned().collect()),
                (d, w) => d.or(w),
            };
            verify(&data, quick, only.as_ref())
        }
        Cmd::Seal { recipient, session, seq, kind, container, r#final, input, output } => {
            let key = srec::seal::parse_recipient(&recipient)?;
            let content = fs::read(&input)?;
            let header = Header {
                v: 1,
                session,
                seq,
                kind,
                container,
                is_final: r#final,
                created_at: time::OffsetDateTime::now_utc()
                    .format(&time::format_description::well_known::Rfc3339)?,
                start_ms: None,
                duration_ms: None,
                device: Some(serde_json::json!({ "platform": "sr-cli" })),
                location: None,
                video: None,
            };
            fs::write(&output, srec::seal::seal(&key, &envelope::encode(&header, &content)))?;
            Ok(())
        }
        Cmd::VerifySig { files } => verify_sig(&files),
        Cmd::Request { code, server, token_file } => {
            let admin = Admin::new(&server, &token_file)?;
            let r = admin.request(&code)?;
            eprintln!(
                "{} -> {} ({} objects, {} bytes){}",
                r["code"].as_str().unwrap_or_default(),
                r["device_name"].as_str().unwrap_or("?"),
                r["objects"],
                r["bytes"],
                if r["cancelled"].as_bool() == Some(true) { "  CANCELLED" } else { "" }
            );
            println!("{}", r["device_id"].as_str().context("no device id")?);
            Ok(())
        }
        Cmd::Share { identity, data, code, server, token_file, days, keep, clipboard } => {
            share(&identity, &data, &code, &server, &token_file, days, keep, clipboard)
        }
        Cmd::Fingerprint { recipient } => {
            srec::seal::parse_recipient(&recipient)?;
            println!("{}", srec::seal::fingerprint(&recipient));
            Ok(())
        }
    }
}

fn keygen(output: Option<PathBuf>, passphrase: bool) -> anyhow::Result<()> {
    match output {
        Some(path) => {
            let pass = if !passphrase {
                None
            } else if let Ok(p) = std::env::var("SR_PASSPHRASE") {
                Some(SecretString::from(p))
            } else {
                let a = rpassword::prompt_password("New passphrase: ")?;
                if a.chars().count() < 8 {
                    bail!("passphrase too short (8 characters minimum)");
                }
                if rpassword::prompt_password("Same passphrase again: ")? != a {
                    bail!("passphrases differ");
                }
                Some(SecretString::from(a))
            };
            let recipient = write_identity(&path, pass.as_ref())?;
            eprintln!("identity written to {}{}", path.display(), if pass.is_some() { " (passphrase-protected)" } else { "" });
            eprintln!("fingerprint: {}", srec::seal::fingerprint(&recipient));
            println!("{recipient}");
        }
        None => {
            let id = age::x25519::Identity::generate();
            let recipient = id.to_public().to_string();
            let now = time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339)?;
            print!(
                "# Témoin identity (PRIVATE KEY) — keep OFFLINE, never on the upload server
# created: {now}
# public key: {recipient}
{}
",
                id.to_string().expose_secret()
            );
        }
    }
    Ok(())
}

fn verify_sig(files: &[PathBuf]) -> anyhow::Result<()> {
    use base64::Engine;
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let b64 = base64::engine::general_purpose::STANDARD;
    let mut bad = 0;
    for f in files {
        let v: serde_json::Value = serde_json::from_slice(&fs::read(f)?)?;
        let key: [u8; 32] = b64.decode(v["public_key"].as_str().unwrap_or_default())?.try_into().map_err(|_| anyhow::anyhow!("key size"))?;
        let sig = Signature::from_slice(&b64.decode(v["signature"].as_str().unwrap_or_default())?)?;
        let msg = v["message"].as_str().unwrap_or_default();
        let ok = VerifyingKey::from_bytes(&key)?.verify(msg.as_bytes(), &sig).is_ok();
        let id_ok = v["device_id"].as_str().is_none_or(|id| id == srec::devices::device_id(&key));
        println!("{} {} (device id {})", if ok { "OK  " } else { "FAIL" }, f.display(), if id_ok { "ok" } else { "MISMATCH" });
        if !ok || !id_ok {
            bad += 1;
        }
    }
    if bad > 0 {
        bail!("{bad} signature vector(s) failed");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn share(
    identity: &Path,
    data: &Path,
    code: &str,
    server: &str,
    token_file: &Path,
    days: u32,
    keep: Option<PathBuf>,
    clipboard: bool,
) -> anyhow::Result<()> {
    let identity = load_identity(identity, passphrase_for(identity)?.as_ref())?;
    let admin = Admin::new(server, token_file)?;
    let link = sr_share::share(&identity, data, &admin, code, days, keep)?;
    sr_share::print_qr(&link);
    if clipboard {
        sr_share::to_clipboard(&link);
    }
    println!("{link}");
    Ok(())
}

/// Passphrase of a protected identity: SR_PASSPHRASE, else asked on the terminal.
fn passphrase_for(identity: &Path) -> anyhow::Result<Option<SecretString>> {
    if !identity_is_protected(identity) {
        return Ok(None);
    }
    if let Ok(p) = std::env::var("SR_PASSPHRASE") {
        return Ok(Some(SecretString::from(p)));
    }
    let p = rpassword::prompt_password(format!("Passphrase for {}: ", identity.display()))?;
    Ok(Some(SecretString::from(p)))
}
