//! Device identities and invitations.
//!
//! Every phone generates its own Ed25519 key pair. It joins the server once with
//! an invitation code (from a QR shown by the operator) and from then on signs
//! every request. The server only ever learns a pseudonym ("Phone #37"), the
//! invitation used and usage counters — never who the person is. Banning a
//! device blocks that key without affecting anybody else.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Accepted clock skew between phone and server for signed requests.
pub const MAX_SKEW_SECS: i64 = 12 * 3600;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub public_key: String,
    pub name: String,
    pub invite_id: String,
    pub created_at: String,
    pub quota_bytes: u64,
    #[serde(default)]
    pub banned: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banned_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invite {
    pub id: String,
    pub code: String,
    pub label: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_uses: Option<u32>,
    #[serde(default)]
    pub uses: u32,
    pub quota_bytes: u64,
    #[serde(default)]
    pub revoked: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    next_number: u32,
    #[serde(default)]
    devices: Vec<Device>,
    #[serde(default)]
    invites: Vec<Invite>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RegisterError {
    #[error("unknown invitation")]
    UnknownInvite,
    #[error("invitation revoked")]
    Revoked,
    #[error("invitation expired")]
    Expired,
    #[error("invitation fully used")]
    Exhausted,
    #[error("bad public key")]
    BadKey,
    #[error("bad signature")]
    BadSignature,
    #[error("device is banned")]
    Banned,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuthError {
    #[error("missing authentication headers")]
    Missing,
    #[error("unknown device")]
    UnknownDevice,
    #[error("device is banned")]
    Banned,
    #[error("request timestamp too far from server time")]
    Skew,
    #[error("bad signature")]
    BadSignature,
}

pub struct Registry {
    path: PathBuf,
    store: Mutex<Store>,
}

/// Device id = first 16 bytes of SHA-256(raw public key), lowercase hex.
pub fn device_id(public_key: &[u8; 32]) -> String {
    hex::encode(&Sha256::digest(public_key)[..16])
}

/// String signed by the device for every authenticated request.
pub fn request_message(method: &str, path: &str, body_sha256: Option<&str>, timestamp: i64, device: &str) -> String {
    format!("SRAUTH1\n{method}\n{path}\n{}\n{timestamp}\n{device}", body_sha256.unwrap_or("-"))
}

/// String signed by the device when it registers with an invitation.
pub fn register_message(invite: &str, public_key_b64: &str, timestamp: i64) -> String {
    format!("SRREG1\n{invite}\n{public_key_b64}\n{timestamp}")
}

fn parse_key(b64: &str) -> Option<(VerifyingKey, [u8; 32])> {
    let raw: [u8; 32] = STANDARD.decode(b64.trim()).ok()?.try_into().ok()?;
    let key = VerifyingKey::from_bytes(&raw).ok()?;
    // Small-order ("weak") keys would let one signature verify for many messages.
    (!key.is_weak()).then_some((key, raw))
}

fn verify(key: &VerifyingKey, msg: &str, sig_b64: &str) -> bool {
    let Ok(bytes) = STANDARD.decode(sig_b64.trim()) else { return false };
    let Ok(sig) = Signature::from_slice(&bytes) else { return false };
    // verify_strict rejects small-order keys and malleable signatures.
    key.verify_strict(msg.as_bytes(), &sig).is_ok()
}

fn now() -> time::OffsetDateTime {
    time::OffsetDateTime::now_utc().replace_nanosecond(0).unwrap()
}

fn rfc3339(t: time::OffsetDateTime) -> String {
    t.format(&time::format_description::well_known::Rfc3339).unwrap()
}

fn parse_rfc3339(s: &str) -> Option<time::OffsetDateTime> {
    time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()
}

/// Human-friendly invitation code: 20 chars from an unambiguous alphabet (~100 bits).
fn new_code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut rng = rand::rngs::OsRng;
    (0..20).map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char).collect()
}

pub struct NewInvite {
    pub label: String,
    pub expires_hours: Option<u32>,
    pub max_uses: Option<u32>,
    pub quota_bytes: u64,
}

impl Registry {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let store = match std::fs::read(path) {
            Ok(b) => serde_json::from_slice(&b)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Store::default(),
            Err(e) => return Err(e.into()),
        };
        Ok(Registry { path: path.to_owned(), store: Mutex::new(store) })
    }

    fn save(&self, store: &Store) -> anyhow::Result<()> {
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(store)?)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    pub fn create_invite(&self, n: NewInvite) -> anyhow::Result<Invite> {
        let mut s = self.store.lock().unwrap();
        let t = now();
        let inv = Invite {
            id: hex::encode(rand::random::<[u8; 4]>()),
            code: new_code(),
            label: n.label.chars().take(80).collect(),
            created_at: rfc3339(t),
            expires_at: n.expires_hours.map(|h| rfc3339(t + time::Duration::hours(h as i64))),
            max_uses: n.max_uses,
            uses: 0,
            quota_bytes: n.quota_bytes,
            revoked: false,
        };
        s.invites.push(inv.clone());
        self.save(&s)?;
        Ok(inv)
    }

    pub fn revoke_invite(&self, id: &str) -> anyhow::Result<bool> {
        let mut s = self.store.lock().unwrap();
        let Some(inv) = s.invites.iter_mut().find(|i| i.id == id) else { return Ok(false) };
        inv.revoked = true;
        self.save(&s)?;
        Ok(true)
    }

    /// True if `code` is (or was) an invitation of this server, whatever its state.
    pub fn invite_known(&self, code: &str) -> bool {
        let code = code.trim();
        !code.is_empty() && self.store.lock().unwrap().invites.iter().any(|i| i.code.eq_ignore_ascii_case(code))
    }

    pub fn invites(&self) -> Vec<Invite> {
        self.store.lock().unwrap().invites.clone()
    }

    pub fn devices(&self) -> Vec<Device> {
        self.store.lock().unwrap().devices.clone()
    }

    pub fn device(&self, id: &str) -> Option<Device> {
        self.store.lock().unwrap().devices.iter().find(|d| d.id == id).cloned()
    }

    pub fn set_banned(&self, id: &str, banned: bool) -> anyhow::Result<bool> {
        let mut s = self.store.lock().unwrap();
        let Some(d) = s.devices.iter_mut().find(|d| d.id == id) else { return Ok(false) };
        d.banned = banned;
        d.banned_at = banned.then(|| rfc3339(now()));
        self.save(&s)?;
        Ok(true)
    }

    pub fn rename(&self, id: &str, name: &str) -> anyhow::Result<bool> {
        let mut s = self.store.lock().unwrap();
        let Some(d) = s.devices.iter_mut().find(|d| d.id == id) else { return Ok(false) };
        d.name = name.trim().chars().take(60).collect();
        self.save(&s)?;
        Ok(true)
    }

    /// Registers (or re-registers, idempotently) a device key with an invitation code.
    pub fn register(&self, code: &str, public_key_b64: &str, timestamp: i64, signature: &str) -> Result<Device, RegisterError> {
        let (key, raw) = parse_key(public_key_b64).ok_or(RegisterError::BadKey)?;
        if (now().unix_timestamp() - timestamp).abs() > MAX_SKEW_SECS
            || !verify(&key, &register_message(code, public_key_b64.trim(), timestamp), signature)
        {
            return Err(RegisterError::BadSignature);
        }
        let id = device_id(&raw);
        let mut s = self.store.lock().unwrap();

        // Same phone scanning again: return its existing identity.
        if let Some(d) = s.devices.iter().find(|d| d.id == id) {
            return if d.banned { Err(RegisterError::Banned) } else { Ok(d.clone()) };
        }

        let inv = s.invites.iter_mut().find(|i| i.code.eq_ignore_ascii_case(code.trim())).ok_or(RegisterError::UnknownInvite)?;
        if inv.revoked {
            return Err(RegisterError::Revoked);
        }
        if inv.expires_at.as_deref().and_then(parse_rfc3339).is_some_and(|e| now() > e) {
            return Err(RegisterError::Expired);
        }
        if inv.max_uses.is_some_and(|m| inv.uses >= m) {
            return Err(RegisterError::Exhausted);
        }
        inv.uses += 1;
        let (invite_id, quota) = (inv.id.clone(), inv.quota_bytes);
        s.next_number += 1;
        let dev = Device {
            id,
            public_key: STANDARD.encode(raw),
            name: format!("Phone #{}", s.next_number),
            invite_id,
            created_at: rfc3339(now()),
            quota_bytes: quota,
            banned: false,
            banned_at: None,
        };
        s.devices.push(dev.clone());
        self.save(&s).map_err(|e| {
            tracing::error!("saving device registry: {e:#}");
            RegisterError::BadKey
        })?;
        Ok(dev)
    }

    /// Verifies a signed request. Returns the device on success.
    pub fn authenticate(
        &self,
        device: Option<&str>,
        timestamp: Option<&str>,
        signature: Option<&str>,
        method: &str,
        path: &str,
        body_sha256: Option<&str>,
    ) -> Result<Device, AuthError> {
        let (Some(device), Some(ts), Some(sig)) = (device, timestamp, signature) else { return Err(AuthError::Missing) };
        let ts: i64 = ts.trim().parse().map_err(|_| AuthError::Missing)?;
        let d = self.device(device).ok_or(AuthError::UnknownDevice)?;
        if (now().unix_timestamp() - ts).abs() > MAX_SKEW_SECS {
            return Err(AuthError::Skew);
        }
        let (key, _) = parse_key(&d.public_key).ok_or(AuthError::UnknownDevice)?;
        if !verify(&key, &request_message(method, path, body_sha256, ts, device), sig) {
            return Err(AuthError::BadSignature);
        }
        if d.banned {
            return Err(AuthError::Banned);
        }
        Ok(d)
    }
}
