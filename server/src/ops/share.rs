//! Hand a person's recordings back through a one-time link (`sr request` / `sr share`,
//! desktop app).

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};

/// Admin API client.
pub struct Admin {
    base: String,
    token: String,
    agent: ureq::Agent,
}

fn read_token(path: &Path) -> anyhow::Result<String> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
    if let Some(v) = lines.iter().find_map(|l| l.strip_prefix("SR_ADMIN_TOKEN=")) {
        return Ok(v.trim().to_string());
    }
    match lines.as_slice() {
        [one] => Ok(one.to_string()),
        _ => bail!("{}: put the admin token alone on one line, or as SR_ADMIN_TOKEN=…", path.display()),
    }
}

impl Admin {
    pub fn new(server: &str, token_file: &Path) -> anyhow::Result<Self> {
        Ok(Self::with_token(server, read_token(token_file)?))
    }

    pub fn with_token(server: &str, token: String) -> Self {
        Admin {
            base: server.trim().trim_end_matches('/').to_string(),
            token,
            agent: ureq::AgentBuilder::new()
                .timeout_connect(std::time::Duration::from_secs(15))
                .timeout_read(std::time::Duration::from_secs(120))
                .timeout_write(std::time::Duration::from_secs(300))
                .build(),
        }
    }

    /// GET on the admin API (desktop app: devices, invitations, requests).
    pub fn get_json(&self, path: &str) -> anyhow::Result<serde_json::Value> {
        self.get(path)
    }

    /// POST to the admin API (also used by the desktop app for invitations).
    pub fn post_json(&self, path: &str, body: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        self.post(path, body)
    }

    fn check(r: Result<ureq::Response, ureq::Error>) -> anyhow::Result<serde_json::Value> {
        match r {
            Ok(resp) => Ok(resp.into_json()?),
            Err(ureq::Error::Status(code, resp)) => {
                let body: serde_json::Value = resp.into_json().unwrap_or_default();
                bail!("server refused ({code}): {}", body["message"].as_str().or(body["error"].as_str()).unwrap_or("no details"))
            }
            Err(e) => bail!("cannot reach the server: {e}"),
        }
    }

    fn auth(&self) -> String {
        format!("Bearer {}", self.token)
    }

    fn get(&self, path: &str) -> anyhow::Result<serde_json::Value> {
        Self::check(self.agent.get(&format!("{}{path}", self.base)).set("Authorization", &self.auth()).call())
    }

    fn post(&self, path: &str, body: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        Self::check(self.agent.post(&format!("{}{path}", self.base)).set("Authorization", &self.auth()).send_json(body))
    }

    fn put_part(&self, id: &str, n: usize, offset: u64, part: &[u8]) -> anyhow::Result<()> {
        let url = format!("{}/api/v1/admin/shares/{id}/files/{n}?offset={offset}", self.base);
        let mut attempt = 0;
        loop {
            let r = self
                .agent
                .put(&url)
                .set("Authorization", &self.auth())
                .set("Content-Type", "application/octet-stream")
                .send_bytes(part);
            match r {
                Ok(_) => return Ok(()),
                // Network trouble or 5xx: the same part is sent again at the same offset.
                Err(e) if attempt < 4 && !matches!(e, ureq::Error::Status(400..=499, _)) => {
                    attempt += 1;
                    eprintln!("\n  retry {attempt} ({e})");
                    std::thread::sleep(std::time::Duration::from_secs(2 * attempt));
                }
                Err(e) => return Self::check(Err(e)).map(|_| ()),
            }
        }
    }

    pub fn request(&self, code: &str) -> anyhow::Result<serde_json::Value> {
        let code: String = code.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        if code.is_empty() {
            bail!("empty request code");
        }
        self.get(&format!("/api/v1/admin/requests/{code}"))
    }
}

/// A decrypted file to hand back.
pub struct Media {
    pub path: PathBuf,
    pub name: String,
    pub mime: &'static str,
    pub kind: String,
    pub created_at: String,
    pub complete: bool,
    pub size: u64,
}

fn safe(s: &str) -> String {
    let out: String = s.chars().filter(|c| c.is_ascii_alphanumeric()).take(16).collect();
    if out.is_empty() { "file".into() } else { out }
}

/// The finished files of a `decrypt` output folder (remuxed MP4 preferred over raw TS).
pub fn collect_media(dir: &Path) -> anyhow::Result<Vec<Media>> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir)? {
        let p = e?.path();
        if p.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let meta: serde_json::Value = serde_json::from_slice(&fs::read(&p)?)?;
        let base = p.file_stem().unwrap().to_string_lossy().to_string();
        let Some((path, ext, mime)) = [("mp4", "video/mp4"), ("jpg", "image/jpeg"), ("heic", "image/heic"), ("ts", "video/mp2t")]
            .iter()
            .map(|(ext, mime)| (dir.join(format!("{base}.{ext}")), *ext, *mime))
            .find(|(p, _, _)| p.exists())
        else {
            continue;
        };
        if ext == "ts" {
            eprintln!("! {base}: no MP4 (ffmpeg missing or failed): sending the raw .ts, iPhones cannot open it");
        }
        // Uploader-controlled: only digits end up in the file name.
        let created_at = meta["created_at"].as_str().unwrap_or_default().to_string();
        let digits: String = created_at.chars().filter(|c| c.is_ascii_digit()).take(14).collect();
        let stem = if digits.len() == 14 { format!("{}-{}", &digits[..8], &digits[8..]) } else { "media".into() };
        out.push(Media {
            size: fs::metadata(&path)?.len(),
            path,
            name: format!("{stem}.{ext}"),
            mime,
            kind: safe(meta["kind"].as_str().unwrap_or("file")),
            created_at,
            complete: meta["complete"].as_bool().unwrap_or(true),
        });
    }
    out.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.name.cmp(&b.name)));
    // Same second twice: number them.
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for m in out.iter_mut() {
        let n = seen.entry(m.name.clone()).or_insert(0);
        *n += 1;
        if *n > 1 {
            let (stem, ext) = m.name.rsplit_once('.').unwrap();
            m.name = format!("{stem}-{n}.{ext}");
        }
    }
    Ok(out)
}

/// Encrypts `reader` (exactly `plain_len` bytes) as file `n` and uploads it in parts.
fn upload_encrypted(
    admin: &Admin,
    id: &str,
    n: usize,
    key: &[u8; 32],
    mut reader: impl Read,
    plain_len: u64,
    progress: &mut dyn FnMut(u64),
) -> anyhow::Result<()> {
    use crate::sharecrypt::{chunk_count, Encryptor, CHUNK, TAG};
    const PART_CHUNKS: usize = 8;
    let mut enc = Encryptor::new(key, n as u32);
    let chunks = chunk_count(plain_len);
    let mut buf = vec![0u8; CHUNK];
    let mut part = Vec::with_capacity(PART_CHUNKS * (CHUNK + TAG));
    let mut offset = 0u64;
    for i in 0..chunks {
        let want = (plain_len - i * CHUNK as u64).min(CHUNK as u64) as usize;
        reader.read_exact(&mut buf[..want]).context("reading a decrypted file")?;
        part.extend_from_slice(&enc.seal_chunk(&buf[..want], i + 1 == chunks));
        if part.len() >= PART_CHUNKS * (CHUNK + TAG) || i + 1 == chunks {
            admin.put_part(id, n, offset, &part)?;
            offset += part.len() as u64;
            progress(part.len() as u64);
            part.clear();
        }
    }
    Ok(())
}

/// Encrypts and uploads `media`; returns the full link (with the key fragment).
pub fn upload(admin: &Admin, code: &str, days: u32, upto: Option<u64>, media: &[Media]) -> anyhow::Result<String> {
    let manifest = serde_json::to_vec(&serde_json::json!({
        "v": 1,
        "files": media.iter().map(|m| serde_json::json!({
            "name": m.name, "type": m.mime, "size": m.size, "kind": m.kind,
            "created_at": m.created_at, "complete": m.complete,
        })).collect::<Vec<_>>(),
    }))?;
    let mut sizes = vec![crate::sharecrypt::ciphertext_len(manifest.len() as u64)];
    sizes.extend(media.iter().map(|m| crate::sharecrypt::ciphertext_len(m.size)));
    let total: u64 = sizes.iter().sum();

    let key = crate::sharecrypt::new_key();
    let created = admin.post("/api/v1/admin/shares", serde_json::json!({ "code": code, "files": sizes, "days": days, "label": code, "upto_index": upto }))?;
    let id = created["id"].as_str().context("no share id")?.to_string();
    let mut sent = 0u64;
    let mut progress = |n: u64| {
        sent += n;
        eprint!("\r  uploading {:>7.1} / {:.1} MB", sent as f64 / 1e6, total as f64 / 1e6);
    };
    upload_encrypted(admin, &id, 0, &key, &manifest[..], manifest.len() as u64, &mut progress)?;
    for (i, m) in media.iter().enumerate() {
        upload_encrypted(admin, &id, i + 1, &key, fs::File::open(&m.path)?, m.size, &mut progress)?;
    }
    eprintln!();
    admin.post(&format!("/api/v1/admin/shares/{id}/seal"), serde_json::Value::Null)?;
    for m in media {
        eprintln!("  {}  {:>8.1} MB{}", m.name, m.size as f64 / 1e6, if m.complete { "" } else { "  (incomplete recording)" });
    }
    eprintln!("{} files. Link valid until {} if not opened; it opens ONCE.", media.len(), created["expires_at"].as_str().unwrap_or("?"));
    Ok(format!("{}#{}", created["link"].as_str().context("no link")?, crate::sharecrypt::key_fragment(&key)))
}

pub fn print_qr(link: &str) {
    if let Ok(qr) = qrcode::QrCode::with_error_correction_level(link.as_bytes(), qrcode::EcLevel::L) {
        use qrcode::render::unicode::Dense1x2;
        let art = qr.render::<Dense1x2>().dark_color(Dense1x2::Light).light_color(Dense1x2::Dark).quiet_zone(true).build();
        eprintln!("\n{art}\n");
    }
}

pub fn to_clipboard(text: &str) {
    let (cmd, args): (&str, &[&str]) = if cfg!(windows) {
        ("clip", &[])
    } else if cfg!(target_os = "macos") {
        ("pbcopy", &[])
    } else {
        ("xclip", &["-selection", "clipboard"])
    };
    let r = std::process::Command::new(cmd).args(args).stdin(std::process::Stdio::piped()).spawn().and_then(|mut child| {
        child.stdin.take().unwrap().write_all(text.as_bytes())?;
        child.wait()
    });
    match r {
        Ok(_) => eprintln!("link copied to the clipboard"),
        Err(e) => eprintln!("! clipboard: {e}"),
    }
}

/// Full hand-back for a request code: decrypt that device's objects from `data`, remux,
/// re-encrypt with a fresh key, upload. Returns the link with its key fragment.
/// The decrypted copies live in `keep` if given, else in a temporary folder deleted after.
pub fn share(
    identity: &age::x25519::Identity,
    data: &Path,
    admin: &Admin,
    code: &str,
    days: u32,
    keep: Option<PathBuf>,
) -> anyhow::Result<String> {
    let req = admin.request(code)?;
    if req["cancelled"].as_bool() == Some(true) {
        bail!("this request was cancelled in /admin");
    }
    let device = req["device_id"].as_str().context("no device id")?.to_string();
    let code = req["code"].as_str().unwrap_or(code).to_string();
    eprintln!("request {code}: {} ({} objects on the server)", req["device_name"].as_str().unwrap_or("?"), req["objects"]);

    let only = super::device_objects(data, &device)?;
    let upto = crate::ledger::verify_file(&super::data_root(data).join("ledger.jsonl"))?
        .iter()
        .filter(|e| e.device == device)
        .map(|e| e.index)
        .max();
    let (work, temporary) = match keep {
        Some(k) => (k, false),
        None => (std::env::temp_dir().join(format!("sr-share-{}", hex::encode(rand::random::<[u8; 6]>()))), true),
    };
    let result = super::decrypt(identity, data, &work, true, false, Some(&only))
        .and_then(|_| collect_media(&work))
        .and_then(|media| {
            if media.is_empty() {
                bail!("nothing to hand back for this device");
            }
            upload(admin, &code, days, upto, &media)
        });
    if temporary && work.exists() {
        // Decrypted copies: do not leave them behind.
        if let Err(e) = fs::remove_dir_all(&work) {
            eprintln!("! could not delete the temporary folder {}: {e}", work.display());
        }
    }
    result
}
