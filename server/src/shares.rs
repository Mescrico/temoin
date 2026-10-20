//! Handing recordings back to the person who filmed them.
//!
//! 1. In the app, the person asks for their files: the server gives the device a short
//!    random **request code** (`K7F3-QX9M`). They pass it to the operator by any channel;
//!    only someone holding the phone can have seen it.
//! 2. On the operator PC (the only place with the private key), `sr share` decrypts that
//!    device's recordings, re-encrypts them with a fresh key ([`crate::sharecrypt`]) and
//!    uploads the result here as a **share**.
//! 3. The link `/r/<id>#<key>` goes to the person over another channel. The server only
//!    ever holds ciphertext; the key stays in the URL fragment.
//!
//! A share opens **once**: the first browser that opens it gets a claim token and
//! [`CLAIM_WINDOW_SECS`] to save the files, then the link stops working. Unopened shares
//! expire after the number of days chosen by the operator.
//!
//! **Nothing is deleted.** A closed share's ciphertext moves to `shares-archive/`, records
//! stay in `shares.json`, and once a link has been retrieved the server moves the
//! recordings it covered to `returned/` (see [`crate::returned`]).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::{Duration, OffsetDateTime};

/// How long a request code stays valid.
pub const REQUEST_DAYS: i64 = 14;
/// Once opened, the share stays downloadable by that browser for this long.
pub const CLAIM_WINDOW_SECS: i64 = 3600;
pub const DEFAULT_SHARE_DAYS: u32 = 7;
pub const MAX_SHARE_DAYS: u32 = 30;
/// An upload not sealed within this delay is abandoned.
const UPLOAD_HOURS: i64 = 24;
pub const MAX_FILES: usize = 5000;
pub const MAX_PART_BYTES: usize = 9 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandRequest {
    pub code: String,
    pub device_id: String,
    pub created_at: String,
    pub expires_at: String,
    #[serde(default)]
    pub cancelled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Being uploaded by `sr share`.
    Uploading,
    /// Link usable, never opened.
    Ready,
    /// Opened by one browser, within its download window.
    Opened,
    /// Opened, then deleted (person confirmed, or window over).
    Done,
    /// Never opened before expiry (or upload abandoned).
    Expired,
    /// Revoked by the operator.
    Revoked,
}

impl State {
    pub fn live(self) -> bool {
        matches!(self, State::Uploading | State::Ready | State::Opened)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Share {
    pub id: String,
    pub created_at: String,
    pub expires_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(default)]
    pub label: String,
    /// Ciphertext size of every file (file 0 = encrypted manifest).
    pub files: Vec<u64>,
    #[serde(default)]
    pub received: Vec<u64>,
    pub state: State,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    claim_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opened_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<String>,
    /// Highest ledger index of the device's objects included in this share.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upto_index: Option<u64>,
    /// The recordings covered by this share were moved to `returned/`.
    #[serde(default)]
    pub returned: bool,
}

impl Share {
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().sum()
    }

    /// End of the download window of an opened share.
    pub fn window_end(&self) -> Option<String> {
        self.opened_at.as_deref().and_then(parse).map(|t| rfc3339(t + Duration::seconds(CLAIM_WINDOW_SECS)))
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    requests: Vec<HandRequest>,
    #[serde(default)]
    shares: Vec<Share>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ShareError {
    #[error("no such share")]
    NotFound,
    #[error("share is still being uploaded")]
    NotReady,
    #[error("this link has already been opened on another device")]
    Used,
    #[error("files already retrieved and deleted")]
    Done,
    #[error("this link has expired")]
    Expired,
    #[error("this link was revoked")]
    Revoked,
    #[error("upload offset mismatch, server has {0} bytes")]
    BadOffset(u64),
    #[error("more data than declared")]
    TooLarge,
    #[error("some files are incomplete")]
    Incomplete,
    #[error("bad file number")]
    BadFile,
    #[error("invalid request: {0}")]
    Invalid(&'static str),
    #[error("storage error: {0}")]
    Io(String),
}

impl From<std::io::Error> for ShareError {
    fn from(e: std::io::Error) -> Self {
        ShareError::Io(e.to_string())
    }
}

pub struct NewShare {
    pub request_code: Option<String>,
    pub upto_index: Option<u64>,
    pub label: String,
    pub files: Vec<u64>,
    pub days: u32,
}

pub struct ShareStore {
    path: PathBuf,
    dir: PathBuf,
    store: Mutex<Store>,
}

fn now() -> OffsetDateTime {
    OffsetDateTime::now_utc().replace_nanosecond(0).unwrap()
}

fn rfc3339(t: OffsetDateTime) -> String {
    t.format(&time::format_description::well_known::Rfc3339).unwrap()
}

fn parse(s: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()
}

fn past(s: &str, t: OffsetDateTime) -> bool {
    parse(s).is_none_or(|e| t > e)
}

/// Request codes: 8 characters from an unambiguous alphabet (40 bits), shown `XXXX-XXXX`.
/// They are only ever typed into the admin side, which is behind the admin token.
fn new_request_code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut rng = rand::rngs::OsRng;
    (0..8).map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char).collect()
}

pub fn normalize_code(code: &str) -> String {
    code.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase()
}

pub fn display_code(code: &str) -> String {
    if code.len() == 8 { format!("{}-{}", &code[..4], &code[4..]) } else { code.to_string() }
}

/// Share ids: 128 random bits, base64url (22 characters).
pub fn valid_id(id: &str) -> bool {
    id.len() == 22 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn hash_claim(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

impl ShareStore {
    pub fn open(data_dir: &Path) -> anyhow::Result<Self> {
        let path = data_dir.join("shares.json");
        let dir = data_dir.join("shares");
        std::fs::create_dir_all(&dir)?;
        let store = match std::fs::read(&path) {
            Ok(b) => serde_json::from_slice(&b)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Store::default(),
            Err(e) => return Err(e.into()),
        };
        let s = ShareStore { path, dir, store: Mutex::new(store) };
        s.sweep();
        Ok(s)
    }

    fn save(&self, store: &Store) -> Result<(), ShareError> {
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(store).map_err(|e| ShareError::Io(e.to_string()))?)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    fn share_dir(&self, id: &str) -> PathBuf {
        self.dir.join(id)
    }

    fn file_path(&self, id: &str, n: usize) -> PathBuf {
        self.share_dir(id).join(format!("{n}.bin"))
    }

    /// Takes the ciphertext of a closed share out of reach of the download routes.
    /// Never deletes: it moves to `shares-archive/<id>`.
    fn archive_content(&self, id: &str) {
        let d = self.share_dir(id);
        if !d.exists() {
            return;
        }
        let archive = self.dir.with_file_name("shares-archive");
        let mut target = archive.join(id);
        let mut n = 1;
        while target.exists() {
            n += 1;
            target = archive.join(format!("{id}-{n}"));
        }
        if let Err(e) = std::fs::create_dir_all(&archive).and_then(|_| std::fs::rename(&d, &target)) {
            tracing::error!("archiving share {id}: {e}");
        }
    }

    /// Retrieved shares whose recordings have not been moved to `returned/` yet:
    /// (share id, device id, highest ledger index included, share creation time).
    pub fn pending_returns(&self) -> Vec<(String, String, Option<u64>, String)> {
        let s = self.store.lock().unwrap();
        s.shares
            .iter()
            .filter(|sh| sh.state == State::Done && !sh.returned)
            .filter_map(|sh| sh.device_id.clone().map(|d| (sh.id.clone(), d, sh.upto_index, sh.created_at.clone())))
            .collect()
    }

    pub fn mark_returned(&self, id: &str) -> Result<(), ShareError> {
        let mut s = self.store.lock().unwrap();
        if let Some(sh) = s.shares.iter_mut().find(|sh| sh.id == id) {
            sh.returned = true;
        }
        self.save(&s)
    }

    /// Time-based transitions. Returns true if the share changed.
    fn refresh(sh: &mut Share, t: OffsetDateTime) -> bool {
        let next = match sh.state {
            State::Uploading if parse(&sh.created_at).is_none_or(|c| t > c + Duration::hours(UPLOAD_HOURS)) => State::Expired,
            State::Ready if past(&sh.expires_at, t) => State::Expired,
            State::Opened if sh.window_end().as_deref().is_none_or(|e| past(e, t)) => State::Done,
            _ => return false,
        };
        sh.state = next;
        sh.closed_at = Some(rfc3339(t));
        true
    }

    // ------------------------------------------------------------ requests

    /// The device's open request, or a new one. An open request is one that has not
    /// expired, was not cancelled and whose share (if any) is still live.
    pub fn request_for_device(&self, device_id: &str) -> Result<HandRequest, ShareError> {
        let mut s = self.store.lock().unwrap();
        let t = now();
        if let Some(r) = Self::open_request(&s, device_id, t) {
            return Ok(r.clone());
        }
        let mut code = new_request_code();
        while s.requests.iter().any(|r| r.code == code) {
            code = new_request_code();
        }
        let r = HandRequest {
            code,
            device_id: device_id.to_string(),
            created_at: rfc3339(t),
            expires_at: rfc3339(t + Duration::days(REQUEST_DAYS)),
            cancelled: false,
            share_id: None,
        };
        s.requests.push(r.clone());
        self.save(&s)?;
        Ok(r)
    }

    fn open_request<'a>(s: &'a Store, device_id: &str, t: OffsetDateTime) -> Option<&'a HandRequest> {
        s.requests.iter().rev().find(|r| {
            r.device_id == device_id
                && !r.cancelled
                && !past(&r.expires_at, t)
                && r.share_id.as_ref().is_none_or(|id| {
                    s.shares.iter().find(|sh| &sh.id == id).is_some_and(|sh| {
                        let mut sh = sh.clone();
                        Self::refresh(&mut sh, t);
                        sh.state.live()
                    })
                })
        })
    }

    /// What the app shows: the latest request of this device and the state of its link.
    pub fn device_status(&self, device_id: &str) -> Option<(HandRequest, Option<State>)> {
        let s = self.store.lock().unwrap();
        let t = now();
        let r = s.requests.iter().rev().find(|r| r.device_id == device_id && !r.cancelled)?;
        let state = r.share_id.as_ref().and_then(|id| s.shares.iter().find(|sh| &sh.id == id)).map(|sh| {
            let mut sh = sh.clone();
            Self::refresh(&mut sh, t);
            sh.state
        });
        // A request that expired without ever getting a link is not worth showing.
        if state.is_none() && past(&r.expires_at, t) {
            return None;
        }
        Some((r.clone(), state))
    }

    pub fn request(&self, code: &str) -> Option<HandRequest> {
        let code = normalize_code(code);
        self.store.lock().unwrap().requests.iter().find(|r| r.code == code).cloned()
    }

    pub fn cancel_request(&self, code: &str) -> Result<bool, ShareError> {
        let code = normalize_code(code);
        let mut s = self.store.lock().unwrap();
        let Some(r) = s.requests.iter_mut().find(|r| r.code == code) else { return Ok(false) };
        r.cancelled = true;
        self.save(&s)?;
        Ok(true)
    }

    pub fn requests(&self) -> Vec<HandRequest> {
        self.store.lock().unwrap().requests.clone()
    }

    // ------------------------------------------------------------ operator side

    pub fn create(&self, n: NewShare) -> Result<Share, ShareError> {
        if n.files.is_empty() || n.files.len() > MAX_FILES {
            return Err(ShareError::Invalid("file count"));
        }
        let mut s = self.store.lock().unwrap();
        let t = now();
        let (request_code, device_id) = match n.request_code.as_deref().map(normalize_code) {
            None => (None, None),
            Some(code) => {
                let r = s.requests.iter().find(|r| r.code == code).ok_or(ShareError::Invalid("unknown request code"))?;
                if r.cancelled {
                    return Err(ShareError::Invalid("request cancelled"));
                }
                (Some(code), Some(r.device_id.clone()))
            }
        };
        let id = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 16]>());
        let days = n.days.clamp(1, MAX_SHARE_DAYS) as i64;
        let share = Share {
            id: id.clone(),
            created_at: rfc3339(t),
            expires_at: rfc3339(t + Duration::days(days)),
            request_code: request_code.clone(),
            device_id,
            label: n.label.chars().take(80).collect(),
            received: vec![0; n.files.len()],
            files: n.files,
            state: State::Uploading,
            claim_hash: None,
            opened_at: None,
            closed_at: None,
            upto_index: n.upto_index,
            returned: false,
        };
        std::fs::create_dir_all(self.share_dir(&id))?;
        if let Some(code) = &request_code {
            // One link per request: a new share replaces the previous one.
            let previous = s.requests.iter().find(|r| &r.code == code).and_then(|r| r.share_id.clone());
            if let Some(prev) = previous {
                if let Some(old) = s.shares.iter_mut().find(|sh| sh.id == prev && sh.state.live()) {
                    old.state = State::Revoked;
                    old.closed_at = Some(rfc3339(t));
                    self.archive_content(&prev);
                }
            }
            if let Some(r) = s.requests.iter_mut().find(|r| &r.code == code) {
                r.share_id = Some(id.clone());
                // Keep the request alive as long as its link.
                r.expires_at = r.expires_at.clone().max(share.expires_at.clone());
            }
        }
        s.shares.push(share.clone());
        self.save(&s)?;
        Ok(share)
    }

    /// Appends `data` to file `n` at `offset` (must equal what is already stored, so a
    /// retried part is detected and the client can resume).
    pub fn append(&self, id: &str, n: usize, offset: u64, data: &[u8]) -> Result<u64, ShareError> {
        let mut s = self.store.lock().unwrap();
        let sh = s.shares.iter_mut().find(|sh| sh.id == id).ok_or(ShareError::NotFound)?;
        if sh.state != State::Uploading {
            return Err(ShareError::Invalid("share is not being uploaded"));
        }
        let declared = *sh.files.get(n).ok_or(ShareError::BadFile)?;
        let path = self.file_path(id, n);
        let have = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        // A retried part may start before the end (the previous attempt wrote some or
        // all of it): cut back to the offset. Never leave a hole.
        if offset > have {
            return Err(ShareError::BadOffset(have));
        }
        if offset + data.len() as u64 > declared {
            return Err(ShareError::TooLarge);
        }
        {
            use std::io::{Seek, SeekFrom, Write};
            let mut f = std::fs::OpenOptions::new().create(true).write(true).truncate(false).open(&path)?;
            f.set_len(offset)?;
            f.seek(SeekFrom::Start(offset))?;
            f.write_all(data)?;
            f.sync_data()?;
        }
        let len = offset + data.len() as u64;
        // Progress hint only (not saved on every part): seal re-checks sizes on disk.
        sh.received[n] = len;
        Ok(len)
    }

    pub fn seal(&self, id: &str) -> Result<Share, ShareError> {
        let mut s = self.store.lock().unwrap();
        let sh = s.shares.iter_mut().find(|sh| sh.id == id).ok_or(ShareError::NotFound)?;
        if sh.state != State::Uploading {
            return Err(ShareError::Invalid("share is not being uploaded"));
        }
        for (n, size) in sh.files.iter().enumerate() {
            let on_disk = std::fs::metadata(self.dir.join(id).join(format!("{n}.bin"))).map(|m| m.len()).unwrap_or(0);
            if on_disk != *size {
                return Err(ShareError::Incomplete);
            }
        }
        sh.received = sh.files.clone();
        sh.state = State::Ready;
        let out = sh.clone();
        self.save(&s)?;
        Ok(out)
    }

    pub fn revoke(&self, id: &str) -> Result<bool, ShareError> {
        let mut s = self.store.lock().unwrap();
        let Some(sh) = s.shares.iter_mut().find(|sh| sh.id == id) else { return Ok(false) };
        if sh.state.live() {
            sh.state = State::Revoked;
            sh.closed_at = Some(rfc3339(now()));
        }
        self.archive_content(id);
        self.save(&s)?;
        Ok(true)
    }

    pub fn shares(&self) -> Vec<Share> {
        let s = self.store.lock().unwrap();
        let t = now();
        s.shares
            .iter()
            .map(|sh| {
                let mut sh = sh.clone();
                Self::refresh(&mut sh, t);
                sh
            })
            .collect()
    }

    // ------------------------------------------------------------ recipient side

    /// Public view of a share (after time-based transitions). `None` if unknown.
    pub fn get(&self, id: &str) -> Option<Share> {
        if !valid_id(id) {
            return None;
        }
        let mut s = self.store.lock().unwrap();
        let t = now();
        let sh = s.shares.iter_mut().find(|sh| sh.id == id)?;
        let changed = Self::refresh(sh, t);
        let out = sh.clone();
        if changed {
            if !out.state.live() {
                self.archive_content(id);
            }
            let _ = self.save(&s);
        }
        Some(out)
    }

    /// Opens the share. The first caller gets a claim token; afterwards only that token
    /// works (so a page reload resumes, but a second device is refused).
    pub fn claim(&self, id: &str, token: Option<&str>) -> Result<(String, Share), ShareError> {
        let mut s = self.store.lock().unwrap();
        let t = now();
        let sh = s.shares.iter_mut().find(|sh| sh.id == id).ok_or(ShareError::NotFound)?;
        let mut dirty = Self::refresh(sh, t);
        let result = match sh.state {
            State::Ready => {
                let token = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 24]>());
                sh.claim_hash = Some(hash_claim(&token));
                sh.state = State::Opened;
                sh.opened_at = Some(rfc3339(t));
                dirty = true;
                Ok((token, sh.clone()))
            }
            State::Opened => match token {
                Some(tok) if sh.claim_hash.as_deref() == Some(hash_claim(tok).as_str()) => Ok((tok.to_string(), sh.clone())),
                _ => Err(ShareError::Used),
            },
            st => Err(Self::closed_error(st)),
        };
        let closed = !sh.state.live();
        if closed {
            self.archive_content(id);
        }
        if dirty {
            self.save(&s)?;
        }
        result
    }

    fn closed_error(st: State) -> ShareError {
        match st {
            State::Uploading => ShareError::NotReady,
            State::Done | State::Opened => ShareError::Done,
            State::Expired => ShareError::Expired,
            State::Revoked => ShareError::Revoked,
            State::Ready => ShareError::NotFound,
        }
    }

    fn check_claim(&self, id: &str, token: &str) -> Result<Share, ShareError> {
        let sh = self.get(id).ok_or(ShareError::NotFound)?;
        match sh.state {
            State::Opened if sh.claim_hash.as_deref() == Some(hash_claim(token).as_str()) => Ok(sh),
            State::Opened | State::Ready => Err(ShareError::Used),
            st => Err(Self::closed_error(st)),
        }
    }

    /// Path and size of file `n`, for the holder of the claim token. File 0 (the encrypted
    /// manifest) is readable before opening, so the page can check the key first.
    pub fn file(&self, id: &str, n: usize, token: Option<&str>) -> Result<(PathBuf, u64), ShareError> {
        let sh = match token {
            None if n == 0 => {
                let sh = self.get(id).ok_or(ShareError::NotFound)?;
                if !matches!(sh.state, State::Ready | State::Opened) {
                    return Err(Self::closed_error(sh.state));
                }
                sh
            }
            None => return Err(ShareError::Used),
            Some(t) => self.check_claim(id, t)?,
        };
        let size = *sh.files.get(n).ok_or(ShareError::BadFile)?;
        Ok((self.file_path(id, n), size))
    }

    /// The person saved everything: delete now.
    pub fn finish(&self, id: &str, token: &str) -> Result<(), ShareError> {
        self.check_claim(id, token)?;
        let mut s = self.store.lock().unwrap();
        if let Some(sh) = s.shares.iter_mut().find(|sh| sh.id == id) {
            sh.state = State::Done;
            sh.closed_at = Some(rfc3339(now()));
        }
        self.archive_content(id);
        self.save(&s)
    }

    /// Applies expiries and archives the content of closed shares.
    /// Returns the number of shares archived.
    pub fn sweep(&self) -> usize {
        let mut s = self.store.lock().unwrap();
        let t = now();
        let mut changed = false;
        let mut archived = 0;
        for sh in s.shares.iter_mut() {
            changed |= Self::refresh(sh, t);
        }
        // Content of closed shares, and directories nobody knows about (crash between
        // mkdir and save), leave the download area.
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if !s.shares.iter().any(|sh| sh.id == name && sh.state.live()) {
                    self.archive_content(&name);
                    archived += 1;
                }
            }
        }
        if changed {
            let _ = self.save(&s);
        }
        archived
    }
}
