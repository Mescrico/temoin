//! Operator-side operations (need the private key): decrypt and reassemble, verify the
//! ledger, hand files back. Used by the `sr` CLI and the desktop app.

pub mod share;

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use age::secrecy::SecretString;
use anyhow::{bail, Context};

use crate::envelope::{self, Header};

/// Loads an identity file: plain text (`AGE-SECRET-KEY-1…` line) or age-encrypted with a
/// passphrase (what `sr keygen --passphrase` and the desktop app write).
pub fn load_identity(path: &Path, passphrase: Option<&SecretString>) -> anyhow::Result<age::x25519::Identity> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let text = if bytes.starts_with(b"age-encryption.org/") {
        let pass = passphrase.context("this identity is protected by a passphrase")?;
        let id = age::scrypt::Identity::new(pass.clone());
        let mut out = String::new();
        age::Decryptor::new(&bytes[..])?
            .decrypt(std::iter::once(&id as &dyn age::Identity))
            .map_err(|_| anyhow::anyhow!("wrong passphrase"))?
            .read_to_string(&mut out)?;
        out
    } else {
        String::from_utf8(bytes).context("identity file is not text")?
    };
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("AGE-SECRET-KEY-1"))
        .context("no AGE-SECRET-KEY-1… line in identity file")?;
    line.parse::<age::x25519::Identity>().map_err(|e| anyhow::anyhow!("bad identity: {e}"))
}

/// Free space on the disk holding `p`.
pub fn available_space(p: &Path) -> Option<u64> {
    fs4::available_space(p).ok()
}

/// True if the identity file needs a passphrase.
pub fn identity_is_protected(path: &Path) -> bool {
    let mut head = [0u8; 19];
    fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).is_ok() && head.starts_with(b"age-encryption.org/")
}

/// A new identity file. With a passphrase, the file is age-encrypted (scrypt): useless
/// without it. Returns the recipient (public key).
pub fn write_identity(path: &Path, passphrase: Option<&SecretString>) -> anyhow::Result<String> {
    use age::secrecy::ExposeSecret;
    let id = age::x25519::Identity::generate();
    let recipient = id.to_public().to_string();
    let now = time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339)?;
    let text = format!(
        "# Témoin identity (PRIVATE KEY) — keep OFFLINE, never on the upload server\n# created: {now}\n# public key: {recipient}\n{}\n",
        id.to_string().expose_secret()
    );
    let content = match passphrase {
        None => text.into_bytes(),
        Some(p) => {
            let enc = age::Encryptor::with_user_passphrase(p.clone());
            let mut out = Vec::new();
            let mut w = enc.wrap_output(&mut out)?;
            w.write_all(text.as_bytes())?;
            w.finish()?;
            out
        }
    };
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path).with_context(|| format!("creating {}", path.display()))?.write_all(&content)?;
    Ok(recipient)
}

pub fn decrypt_object(id: &age::x25519::Identity, path: &Path) -> anyhow::Result<Vec<u8>> {
    let bytes = fs::read(path)?;
    let decryptor = age::Decryptor::new(&bytes[..])?;
    let mut reader = decryptor.decrypt(std::iter::once(id as &dyn age::Identity))?;
    let mut out = Vec::new();
    reader.read_to_end(&mut out)?;
    Ok(out)
}

pub fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for e in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let e = e?;
        let p = e.path();
        if e.file_type()?.is_dir() {
            if p.file_name().is_some_and(|n| n == ".tmp") {
                continue;
            }
            walk(&p, out)?;
        } else if p.extension().is_some_and(|x| x == "age") {
            out.push(p);
        }
    }
    Ok(())
}

pub fn decrypt(
    id: &age::x25519::Identity,
    data: &Path,
    output: &Path,
    remux: bool,
    skip_existing: bool,
    only: Option<&std::collections::HashSet<(String, u32)>>,
) -> anyhow::Result<DecryptReport> {
    let mut files = Vec::new();
    walk(data, &mut files)?;
    if let Some(keep) = only {
        files.retain(|f| stored_location(f).is_some_and(|loc| keep.contains(&loc)));
    }
    // The same object may sit in a local copy twice (objects/ and, once handed back, returned/).
    let mut seen = std::collections::HashSet::new();
    files.sort();
    files.retain(|f| stored_location(f).is_none_or(|loc| seen.insert(loc)));
    if files.is_empty() {
        bail!("no .age objects to decrypt under {}", data.display());
    }
    fs::create_dir_all(output)?;

    // Pass 1: read every header (content is discarded to keep memory flat).
    //
    // Everything inside an object was written by the uploader and must be treated as
    // hostile: grouping and ordering use the location the *server* chose
    // (objects/xx/<session>/<seq>.age); the encrypted header is only metadata, and
    // every value that ends up in a file name is sanitized.
    let mut sessions: BTreeMap<String, Vec<(Header, PathBuf, u32)>> = BTreeMap::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut failed = 0usize;
    for (i, f) in files.iter().enumerate() {
        match decrypt_object(id, f).and_then(|pt| Ok(envelope::decode(&pt)?.0)) {
            Ok(h) => {
                let (session, seq) = stored_location(f).unwrap_or_else(|| (h.session.clone(), h.seq));
                if h.session != session || h.seq != seq {
                    warnings.push(format!(
                        "{}: header claims session {:?} seq {} (stored as {session} / {seq})",
                        f.display(),
                        h.session.chars().take(80).collect::<String>(),
                        h.seq
                    ));
                }
                sessions.entry(safe_name(&session, 64)).or_default().push((h, f.clone(), seq));
            }
            Err(e) => {
                failed += 1;
                eprintln!("! {}: {e:#}", f.display());
            }
        }
        if (i + 1) % 200 == 0 {
            eprintln!("  scanned {}/{}", i + 1, files.len());
        }
    }

    // Pass 2: reassemble each session.
    let mut report = Vec::new();
    for (session, mut parts) in sessions {
        parts.sort_by_key(|(_, _, seq)| *seq);
        let first = &parts[0].0;
        let ext = match first.container.as_str() {
            "mpegts" => "ts",
            "fmp4-init" | "fmp4-media" => "mp4",
            "jpeg" => "jpg",
            "heic" => "heic",
            _ => "bin",
        };
        // created_at is uploader-controlled: keep digits only for the file name.
        let digits: String = first.created_at.chars().filter(|c| c.is_ascii_digit()).take(14).collect();
        let stamp = if digits.len() == 14 { format!("{}-{}", &digits[..8], &digits[8..]) } else { "undated".into() };
        let base = format!("{stamp}_{}_{session}", safe_name(&first.kind, 16));
        let media_path = output.join(format!("{base}.{ext}"));
        let meta_path = output.join(format!("{base}.json"));
        debug_assert!(media_path.parent() == Some(output));

        let seqs: Vec<u32> = parts.iter().map(|(_, _, seq)| *seq).collect();
        let missing = missing_seqs(&seqs, 50);
        let complete = missing.is_empty() && parts.last().unwrap().0.is_final;

        if !(skip_existing && media_path.exists()) {
            let mut out = fs::File::create(&media_path)?;
            for (_, path, _) in &parts {
                let pt = decrypt_object(id, path)?;
                let (_, content) = envelope::decode(&pt)?;
                out.write_all(content)?;
            }
            out.sync_all()?;
            let meta = serde_json::json!({
                "session": session,
                "kind": first.kind,
                "container": first.container,
                "created_at": first.created_at,
                "objects": parts.len(),
                "missing_seq": missing,
                "complete": complete,
                "device": first.device,
                "video": first.video,
                "locations": parts.iter().filter_map(|(h, _, _)| h.location.clone()).collect::<Vec<_>>(),
                "segments": parts.iter().map(|(h, p, seq)| serde_json::json!({
                    "seq": seq, "created_at": h.created_at, "start_ms": h.start_ms,
                    "duration_ms": h.duration_ms, "final": h.is_final,
                    "object": p.strip_prefix(data).unwrap_or(p).display().to_string(),
                })).collect::<Vec<_>>(),
            });
            fs::write(&meta_path, serde_json::to_vec_pretty(&meta)?)?;

            if remux && (ext == "ts" || ext == "mp4") {
                // MPEG-TS has no rotation flag; the app records the clockwise rotation
                // needed to show the picture upright, MP4 stores it as display matrix.
                // Timestamps are rebased to 0 (iOS 0.2.1 kept the phone's uptime clock).
                let rotation = first
                    .video
                    .as_ref()
                    .and_then(|v| v.get("rotation"))
                    .and_then(|r| r.as_i64())
                    .unwrap_or(0);
                let target = output.join(format!("{base}.remux.mp4"));
                let mut cmd = std::process::Command::new("ffmpeg");
                cmd.args(["-loglevel", "error", "-y", "-fflags", "+genpts"]);
                if rotation % 360 != 0 {
                    cmd.args(["-display_rotation:v:0", &(-rotation).to_string()]);
                }
                let status = cmd
                    .arg("-i")
                    .arg(&media_path)
                    .args(["-c", "copy", "-avoid_negative_ts", "make_zero", "-movflags", "+faststart"])
                    .arg(&target)
                    .status();
                match status {
                    Ok(s) if s.success() => {
                        // For iOS (.mp4) this replaces the raw fMP4 concatenation by the clean file.
                        fs::rename(&target, output.join(format!("{base}.mp4")))?;
                    }
                    Ok(s) => eprintln!("! ffmpeg failed ({s}) for {}", media_path.display()),
                    Err(e) => eprintln!("! ffmpeg not available: {e}"),
                }
            }
        }
        report.push(format!(
            "{} {:<5} {:>4} obj {}{}",
            if complete { "✓" } else { "…" },
            first.kind,
            parts.len(),
            media_path.file_name().unwrap().to_string_lossy(),
            if missing.is_empty() { String::new() } else { format!("  (missing seq {missing:?})") }
        ));
    }
    for w in warnings.iter().take(100) {
        eprintln!("! {w}");
    }
    Ok(DecryptReport { sessions: report.len(), objects: files.len() - failed, unreadable: failed, lines: report })
}

/// What `decrypt` produced.
#[derive(Debug, Default)]
pub struct DecryptReport {
    pub sessions: usize,
    pub objects: usize,
    pub unreadable: usize,
    /// One line per reassembled photo/video.
    pub lines: Vec<String>,
}

impl DecryptReport {
    pub fn print(&self) {
        for line in &self.lines {
            println!("{line}");
        }
        println!("{} sessions, {} objects, {} unreadable", self.sessions, self.objects, self.unreadable);
    }
}

pub fn verify(data: &Path, quick: bool, only: Option<&std::collections::HashSet<(String, u32)>>) -> anyhow::Result<()> {
    use sha2::{Digest, Sha256};
    let ledger = data.join("ledger.jsonl");
    let entries = crate::ledger::verify_file(&ledger)?;
    println!("ledger chain OK: {} entries", entries.len());
    if let Some(last) = entries.last() {
        println!("head: {}", last.chain);
    }
    if quick {
        return Ok(());
    }
    let mut bad = 0;
    let mut checked = 0;
    for e in &entries {
        if only.is_some_and(|o| !o.contains(&(e.session.clone(), e.seq))) {
            continue;
        }
        checked += 1;
        let rel = Path::new(&e.session[..2]).join(&e.session).join(format!("{:06}.age", e.seq));
        let mut p = data.join("objects").join(&rel);
        if !p.exists() && data.join("returned").join(&rel).exists() {
            p = data.join("returned").join(&rel);
        }
        match fs::read(&p) {
            Ok(b) if hex::encode(Sha256::digest(&b)) == e.sha256 => {}
            Ok(_) => {
                bad += 1;
                eprintln!("! hash mismatch: {}", p.display());
            }
            Err(err) => {
                bad += 1;
                eprintln!("! missing {}: {err}", p.display());
            }
        }
    }
    if bad > 0 {
        bail!("{bad} objects missing or altered");
    }
    println!("all {checked} objects match their receipts");
    Ok(())
}

/// Session and sequence chosen by the server: objects/<xx>/<session>/<seq>.age.
pub fn stored_location(path: &Path) -> Option<(String, u32)> {
    let seq = path.file_stem()?.to_str()?.parse().ok()?;
    let session = path.parent()?.file_name()?.to_str()?.to_string();
    valid_session(&session).then_some((session, seq))
}

/// Keeps a value usable in a file name whatever the uploader put in it.
pub fn safe_name(s: &str, max: usize) -> String {
    let out: String = s
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(max)
        .collect();
    if out.is_empty() { "x".into() } else { out }
}

/// Missing sequence numbers between 0 and the highest one, at most `limit` listed
/// (a hostile seq like 4 000 000 000 must not make this loop forever).
pub fn missing_seqs(sorted: &[u32], limit: usize) -> Vec<u32> {
    let mut out = Vec::new();
    let mut expected = 0u64;
    for &s in sorted {
        let s = s as u64;
        while expected < s && out.len() < limit {
            out.push(expected as u32);
            expected += 1;
        }
        if out.len() >= limit {
            break;
        }
        expected = expected.max(s + 1);
    }
    out
}


/// The data directory itself, whether `data` points at it or at its `objects/` folder.
pub fn data_root(data: &Path) -> PathBuf {
    if data.join("ledger.jsonl").exists() {
        data.to_owned()
    } else {
        data.parent().map(Path::to_owned).unwrap_or_else(|| data.to_owned())
    }
}

/// Resolves a device key prefix to the (session, seq) of all its stored objects.
pub fn device_objects(data: &Path, key: &str) -> anyhow::Result<std::collections::HashSet<(String, u32)>> {
    let prefix: String = key.chars().filter(|c| c.is_ascii_hexdigit()).collect::<String>().to_ascii_lowercase();
    if prefix.len() < 8 {
        bail!("--device needs at least 8 hex characters of the device key");
    }
    let root = data_root(data);
    // The ledger maps every object to the device that uploaded it; refuse a tampered one.
    let entries = crate::ledger::verify_file(&root.join("ledger.jsonl"))?;
    let ids: std::collections::BTreeSet<&str> =
        entries.iter().map(|e| e.device.as_str()).filter(|d| d.starts_with(&prefix)).collect();
    let id = match ids.len() {
        0 => bail!("no uploads from a device whose key starts with {prefix}"),
        1 => *ids.iter().next().unwrap(),
        _ => bail!("{prefix} matches several devices, give more characters"),
    };
    let set: std::collections::HashSet<(String, u32)> =
        entries.iter().filter(|e| e.device == id).map(|e| (e.session.clone(), e.seq)).collect();
    let name = crate::devices::Registry::open(&root.join("devices.json"))
        .ok()
        .and_then(|r| r.device(id))
        .map(|d| d.name)
        .unwrap_or_default();
    eprintln!("device {} {name}: {} objects", group_key(id), set.len());
    Ok(set)
}

/// The objects of every session that received something at or after ledger index `since`.
/// Built from the verified ledger only (server-written); session names are re-validated
/// before they end up in a path.
pub struct Window {
    pub since: u64,
    pub next: u64,
    pub ordered: Vec<(String, u32)>,
    pub objects: std::collections::HashSet<(String, u32)>,
}

impl Window {
    pub fn resolve(data: &Path, since: Option<u64>, state: Option<&Path>) -> anyhow::Result<Option<Window>> {
        let since = match (since, state) {
            (Some(n), _) => n,
            (None, Some(f)) => match fs::read_to_string(f) {
                Ok(t) => t.trim().parse().with_context(|| format!("{}: not an index", f.display()))?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
                Err(e) => return Err(e).with_context(|| format!("reading {}", f.display())),
            },
            (None, None) => return Ok(None),
        };
        let entries = crate::ledger::verify_file(&data_root(data).join("ledger.jsonl"))?;
        let next = entries.last().map_or(0, |e| e.index + 1);
        if since > next {
            bail!("index {since} is beyond the ledger end ({next}): wrong data directory or state file?");
        }
        let touched: std::collections::HashSet<&str> =
            entries.iter().filter(|e| e.index >= since).map(|e| e.session.as_str()).collect();
        let mut ordered = Vec::new();
        let mut objects = std::collections::HashSet::new();
        for e in &entries {
            if !touched.contains(e.session.as_str()) || !valid_session(&e.session) {
                continue;
            }
            if objects.insert((e.session.clone(), e.seq)) {
                ordered.push((e.session.clone(), e.seq));
            }
        }
        Ok(Some(Window { since, next, ordered, objects }))
    }

    pub fn save(&self, state: &Path) -> anyhow::Result<()> {
        let tmp = state.with_extension("tmp");
        fs::write(&tmp, format!("{}\n", self.next))?;
        fs::rename(&tmp, state)?;
        eprintln!("next run starts at ledger index {}", self.next);
        Ok(())
    }
}

pub fn valid_session(s: &str) -> bool {
    (8..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn group_key(id: &str) -> String {
    id.as_bytes().chunks(4).take(4).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join(" ")
}

pub fn list_devices(data: &Path) -> anyhow::Result<()> {
    let root = data_root(data);
    let reg = crate::devices::Registry::open(&root.join("devices.json"))?;
    let invites = reg.invites();
    let entries = crate::ledger::verify_file(&root.join("ledger.jsonl")).unwrap_or_default();
    println!("{:<20} {:<22} {:>7} {:>10}  {:<20}  {:<16} status", "key", "name", "objects", "bytes", "last upload", "invitation");
    for d in reg.devices() {
        let mine: Vec<_> = entries.iter().filter(|e| e.device == d.id).collect();
        let bytes: u64 = mine.iter().map(|e| e.size).sum();
        let last = mine.last().map(|e| e.received_at.clone()).unwrap_or_else(|| "-".into());
        let invite = invites.iter().find(|i| i.id == d.invite_id).map(|i| i.label.clone()).unwrap_or_default();
        println!(
            "{:<20} {:<22} {:>7} {:>10}  {:<20}  {:<16} {}",
            group_key(&d.id),
            d.name.chars().take(22).collect::<String>(),
            mine.len(),
            bytes,
            last,
            invite.chars().take(16).collect::<String>(),
            if d.banned { "banned" } else { "active" }
        );
    }
    Ok(())
}

/// Objects already handed back (moved to returned/ on the server), from `returned.jsonl`.
pub fn returned_set(data: &Path) -> std::collections::HashSet<(String, u32)> {
    let Ok(text) = fs::read_to_string(data_root(data).join("returned.jsonl")) else { return Default::default() };
    text.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| Some((v["session"].as_str()?.to_string(), u32::try_from(v["seq"].as_u64()?).ok()?)))
        .filter(|(s, _)| valid_session(s))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_sanitized() {
        assert_eq!(safe_name("../../etc/passwd", 64), "etcpasswd");
        assert_eq!(safe_name("C:\\Windows\\x", 64), "CWindowsx");
        assert_eq!(safe_name("\0\u{202e}", 64), "x");
        assert_eq!(safe_name(&"a".repeat(500), 64).len(), 64);
    }

    #[test]
    fn missing_is_bounded() {
        assert_eq!(missing_seqs(&[0, 1, 3], 50), vec![2]);
        assert_eq!(missing_seqs(&[0, u32::MAX], 50).len(), 50);
        assert!(missing_seqs(&[0, 1, 2], 50).is_empty());
    }

    #[test]
    fn location_comes_from_path() {
        let p = Path::new("data/objects/ab/abcdefgh1234/000007.age");
        assert_eq!(stored_location(p), Some(("abcdefgh1234".into(), 7)));
        assert_eq!(stored_location(Path::new("x/../000001.age")), None);
    }
}
