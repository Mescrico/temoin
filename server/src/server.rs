//! HTTP surface. There is deliberately no route that reads, lists or deletes objects.
//! The only downloads are hand-back links (`/r/<id>`): ciphertext produced on the
//! operator PC with a key the server never sees (see [`crate::shares`]).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use axum::body::Bytes;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Path, Query, Request, State};
use axum::http::{header, HeaderMap, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::sync::Mutex;

use crate::config::Config;
use crate::devices::{AuthError, NewInvite, RegisterError, Registry};
use crate::events::{Event, EventLog};
use crate::ledger::Ledger;
use crate::returned::Returned;
use crate::shares::{self, NewShare, ShareError, ShareStore};
use crate::{ageformat, seal};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const PROTOCOL: u32 = 1;

pub struct AppState {
    pub cfg: Config,
    pub ledger: Mutex<Ledger>,
    pub accepting: AtomicBool,
    pub started: Instant,
    pub events: EventLog,
    pub registry: Registry,
    pub shares: ShareStore,
    pub returned: std::sync::Mutex<Returned>,
    /// Current public URL: SR_PUBLIC_URL, or the address of a running tunnel.
    public_url: std::sync::RwLock<Option<String>>,
    tunnel: std::sync::RwLock<crate::tunnel::TunnelStatus>,
    objects_dir: PathBuf,
    tmp_dir: PathBuf,
    state_file: PathBuf,
}

#[derive(Serialize, Deserialize, Default)]
struct PersistedState {
    accepting: Option<bool>,
}

impl AppState {
    pub fn open(cfg: Config) -> anyhow::Result<Arc<Self>> {
        let objects_dir = cfg.data_dir.join("objects");
        let tmp_dir = cfg.data_dir.join(".tmp");
        std::fs::create_dir_all(&objects_dir)?;
        std::fs::create_dir_all(&tmp_dir)?;
        // Leftovers of interrupted uploads.
        for e in std::fs::read_dir(&tmp_dir)?.flatten() {
            let _ = std::fs::remove_file(e.path());
        }
        let state_file = cfg.data_dir.join("state.json");
        let persisted: PersistedState = std::fs::read(&state_file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let ledger = Ledger::open(&cfg.data_dir.join("ledger.jsonl"))?;
        let events = EventLog::open(cfg.data_dir.join("logs"))?;
        let registry = Registry::open(&cfg.data_dir.join("devices.json"))?;
        let shares = ShareStore::open(&cfg.data_dir)?;
        let returned = std::sync::Mutex::new(Returned::open(&cfg.data_dir)?);
        events.record(Event {
            ts: now_rfc3339_ms(),
            method: "START".into(),
            path: format!("temoin-server {VERSION}"),
            status: 0,
            ip: None,
            device: None,
            bytes: None,
            error: None,
            index: None,
            note: Some(format!(
                "ledger objects={} devices={} accepting={}",
                ledger.next_index,
                registry.devices().len(),
                persisted.accepting.unwrap_or(cfg.accepting_default)
            )),
            ms: 0,
            ua: None,
        });
        Ok(Arc::new(AppState {
            events,
            registry,
            shares,
            returned,
            public_url: std::sync::RwLock::new(cfg.public_url.clone()),
            tunnel: std::sync::RwLock::new(crate::tunnel::TunnelStatus {
                mode: cfg.tunnel.clone(),
                state: if cfg.tunnel == "off" { "off".into() } else { "starting".into() },
                ..Default::default()
            }),
            accepting: AtomicBool::new(persisted.accepting.unwrap_or(cfg.accepting_default)),
            cfg,
            ledger: Mutex::new(ledger),
            started: Instant::now(),
            objects_dir,
            tmp_dir,
            state_file,
        }))
    }

    /// Base URL phones use (invitations, hand-back links).
    pub fn public_url(&self) -> String {
        self.public_url
            .read()
            .unwrap()
            .clone()
            .unwrap_or_else(|| format!("http://{}", self.cfg.bind))
    }

    pub fn set_public_url(&self, url: Option<String>) {
        let url = url.map(|u| u.trim_end_matches('/').to_string());
        let mut cur = self.public_url.write().unwrap();
        if *cur != url {
            if let Some(u) = &url {
                self.events.record(Event {
                    ts: now_rfc3339_ms(),
                    method: "URL".into(),
                    path: u.chars().take(200).collect(),
                    status: 0,
                    ip: None,
                    device: None,
                    bytes: None,
                    error: None,
                    index: None,
                    note: Some("public address changed: phones must re-scan an invitation".into()),
                    ms: 0,
                    ua: None,
                });
            }
            *cur = url;
        }
    }

    pub fn tunnel(&self) -> crate::tunnel::TunnelStatus {
        self.tunnel.read().unwrap().clone()
    }

    pub fn set_tunnel(&self, t: crate::tunnel::TunnelStatus) {
        *self.tunnel.write().unwrap() = t;
    }

    fn free_bytes(&self) -> Option<u64> {
        fs4::available_space(&self.cfg.data_dir).ok()
    }

    /// Bytes / objects of a device still counted against its quota (handed-back ones are not).
    pub fn device_usage(&self, l: &Ledger, device: &str) -> (u64, u64) {
        let r = self.returned.lock().unwrap();
        (
            l.device_bytes.get(device).copied().unwrap_or(0).saturating_sub(r.device_bytes(device)),
            l.device_objects.get(device).copied().unwrap_or(0).saturating_sub(r.device_objects(device)),
        )
    }

    /// Moves the recordings of retrieved hand-back links to `returned/` (blocking I/O).
    /// Nothing is deleted; the device quota no longer counts them.
    pub fn process_returns(&self) {
        let pending = self.shares.pending_returns();
        if pending.is_empty() {
            return;
        }
        let entries = match crate::ledger::read_entries(&self.cfg.data_dir.join("ledger.jsonl")) {
            Ok(e) => e,
            Err(e) => return tracing::error!("reading ledger for hand-back: {e:#}"),
        };
        for (id, device, upto, created_at) in pending {
            // Links made before upto_index existed: what the device had sent by then.
            let upto = upto.or_else(|| {
                entries.iter().filter(|e| e.device == device && e.received_at <= created_at).map(|e| e.index).max()
            });
            let Some(upto) = upto else {
                let _ = self.shares.mark_returned(&id);
                continue;
            };
            let moved = self.returned.lock().unwrap().move_device(&entries, &device, upto, &id, &now_rfc3339());
            match moved {
                Ok((n, bytes)) => {
                    if let Err(e) = self.shares.mark_returned(&id) {
                        tracing::error!("hand-back {id}: {e}");
                    }
                    tracing::info!(objects = n, bytes, "recordings handed back moved to returned/");
                    self.events.record(Event {
                        ts: now_rfc3339_ms(),
                        method: "MOVE".into(),
                        path: "returned/".into(),
                        status: 0,
                        ip: None,
                        device: Some(device.chars().take(32).collect()),
                        bytes: Some(bytes),
                        error: None,
                        index: None,
                        note: Some(format!("{n} objects handed back, quota freed")),
                        ms: 0,
                        ua: None,
                    });
                }
                Err(e) => tracing::error!("moving handed-back recordings of share {id}: {e:#}"),
            }
        }
    }

    /// QR payload for an invitation: server URL + pinned key + invitation code.
    pub fn enroll_link(&self, invite_code: &str) -> String {
        let url = self.public_url();
        format!(
            "srec://enroll?u={}&k={}&i={}&n={}",
            urlencoding::encode(&url),
            urlencoding::encode(&self.cfg.recipient),
            urlencoding::encode(invite_code),
            urlencoding::encode(&self.cfg.name)
        )
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    let limit = state.cfg.max_object_bytes as usize + 1024;
    Router::new()
        .route("/", get(landing))
        .route("/healthz", get(|| async { "ok" }))
        .route("/api/v1/info", get(info))
        .route(
            "/api/v1/objects/{session}/{seq}",
            put(put_object).layer(DefaultBodyLimit::max(limit)),
        )
        .route("/api/v1/devices", post(register_device))
        .route("/api/v1/me", get(me))
        .route("/api/v1/me/request", post(me_request))
        .route("/r/{id}", get(share_page))
        .route("/r/{id}/meta", get(share_meta))
        .route("/r/{id}/open", post(share_open))
        .route("/r/{id}/f/{n}", get(share_file))
        .route("/r/{id}/done", post(share_done))
        .route("/api/v1/admin/stats", get(admin_stats))
        .route("/api/v1/admin/accepting", post(admin_accepting_logged))
        .route("/api/v1/admin/invites", get(admin_invites).post(admin_create_invite))
        .route("/api/v1/admin/invites/qr", get(admin_invite_qr))
        .route("/api/v1/admin/invites/{id}/revoke", post(admin_revoke_invite))
        .route("/api/v1/admin/devices", get(admin_devices))
        .route("/api/v1/admin/devices/{id}", post(admin_update_device))
        .route("/api/v1/admin/events", get(admin_events))
        .route("/api/v1/admin/requests", get(admin_requests))
        .route("/api/v1/admin/requests/{code}", get(admin_request))
        .route("/api/v1/admin/requests/{code}/objects", get(admin_request_objects))
        .route("/api/v1/admin/objects/{session}/{seq}", get(admin_object))
        .route("/api/v1/admin/handback.js", get(admin_handback_js))
        .route("/api/v1/admin/requests/{code}/cancel", post(admin_cancel_request))
        .route("/api/v1/admin/shares", post(admin_create_share))
        .route(
            "/api/v1/admin/shares/{id}/files/{n}",
            put(admin_share_part).layer(DefaultBodyLimit::max(shares::MAX_PART_BYTES + 1024)),
        )
        .route("/api/v1/admin/shares/{id}/seal", post(admin_seal_share))
        .route("/api/v1/admin/shares/{id}/revoke", post(admin_revoke_share))
        .route(&state.cfg.admin_path.clone(), get(admin_page))
        .fallback(|| async { hidden() })
        // Whole-request deadline: a client trickling bytes (slowloris) cannot hold a
        // connection forever. 5 min leaves room for 16 MB on a poor mobile link.
        .layer(tower_http::timeout::TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, std::time::Duration::from_secs(state.cfg.request_timeout_secs)))
        .layer(middleware::from_fn_with_state(state.clone(), journal))
        .with_state(state)
}

// ---------------------------------------------------------------- journal

/// Error code attached to error responses, picked up by the journal.
#[derive(Clone, Copy)]
struct ErrorCode(&'static str);
/// Free-form note for the journal (admin actions).
#[derive(Clone)]
struct Note(String);
/// Ledger index of a stored object, picked up by the journal.
#[derive(Clone, Copy)]
struct StoredIndex(u64);

fn client_ip(st: &AppState, req: &Request) -> Option<String> {
    if !st.cfg.log_ips {
        return None;
    }
    let h = req.headers();
    if st.cfg.trust_proxy {
        // Set by cloudflared (quick tunnel) or the reverse proxy chain (Traefik -> nginx -> here).
        if let Some(ip) = h.get("cf-connecting-ip").or_else(|| h.get("x-real-ip")).and_then(|v| v.to_str().ok()) {
            return Some(ip.trim().to_string());
        }
        if let Some(xff) = h.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
            if let Some(first) = xff.split(',').next() {
                return Some(first.trim().to_string());
            }
        }
    }
    req.extensions()
        .get::<ConnectInfo<std::net::SocketAddr>>()
        .map(|c| c.0.ip().to_string())
}

/// Logs every request (except the container health check) to the event journal.
async fn journal(State(st): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    if path == "/healthz" {
        return next.run(req).await;
    }
    let started = Instant::now();
    let method = req.method().to_string();
    let ip = client_ip(&st, &req);
    let device = req.headers().get("x-sr-device").and_then(|v| v.to_str().ok()).map(|d| d.chars().take(32).collect());
    let ua = req.headers().get(header::USER_AGENT).and_then(|v| v.to_str().ok()).map(|u| u.chars().take(120).collect());
    let bytes = req.headers().get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok());

    let mut resp = next.run(req).await;
    resp.headers_mut().insert("x-robots-tag", header::HeaderValue::from_static("noindex, nofollow"));

    let ev = Event {
        ts: now_rfc3339_ms(),
        method,
        path: path.chars().take(200).collect(),
        status: resp.status().as_u16(),
        ip,
        device,
        bytes,
        error: resp.extensions().get::<ErrorCode>().map(|e| e.0.to_string()),
        index: resp.extensions().get::<StoredIndex>().map(|i| i.0),
        note: resp.extensions().get::<Note>().map(|n| n.0.clone()),
        ms: started.elapsed().as_millis() as u64,
        ua,
    };
    tracing::info!(
        ip = ev.ip.as_deref().unwrap_or("-"),
        status = ev.status,
        error = ev.error.as_deref().unwrap_or("-"),
        bytes = ev.bytes.unwrap_or(0),
        ms = ev.ms,
        "{} {}", ev.method, ev.path
    );
    st.events.record(ev);
    resp
}

fn now_rfc3339_ms() -> String {
    let t = time::OffsetDateTime::now_utc();
    let t = t.replace_nanosecond(t.millisecond() as u32 * 1_000_000).unwrap();
    t.format(&time::format_description::well_known::Rfc3339).unwrap()
}

// ---------------------------------------------------------------- errors

pub struct ApiError(StatusCode, &'static str, String);

impl ApiError {
    fn new(code: StatusCode, error: &'static str, msg: impl Into<String>) -> Self {
        ApiError(code, error, msg.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if self.1 == "hidden" {
            return hidden();
        }
        let mut r = (self.0, Json(json!({ "error": self.1, "message": self.2 }))).into_response();
        r.extensions_mut().insert(ErrorCode(self.1));
        r
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        tracing::error!("internal error: {e:#}");
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", "internal error")
    }
}

/// A bare 404, identical to an unknown path: reveals nothing about the server.
fn hidden() -> Response {
    let mut r = StatusCode::NOT_FOUND.into_response();
    r.extensions_mut().insert(ErrorCode("hidden"));
    r
}

// ---------------------------------------------------------------- public

async fn landing(State(st): State<Arc<AppState>>) -> Response {
    if st.cfg.stealth {
        return hidden();
    }
    Html(LANDING_HTML.replace("{{NAME}}", &html_escape(&st.cfg.name))).into_response()
}

/// Server description. In stealth mode only for a phone holding an invitation code
/// (`X-SR-Invite`) or a registered device key (signed request, even if banned).
async fn info(State(st): State<Arc<AppState>>, uri: Uri, headers: HeaderMap) -> Response {
    if st.cfg.stealth {
        let invited = header(&headers, "x-sr-invite").is_some_and(|c| st.registry.invite_known(&c.replace([' ', '-'], "")));
        let device = matches!(
            st.registry.authenticate(
                header(&headers, "x-sr-device"),
                header(&headers, "x-sr-timestamp"),
                header(&headers, "x-sr-signature"),
                "GET",
                uri.path(),
                None,
            ),
            Ok(_) | Err(AuthError::Banned)
        );
        if !invited && !device {
            return hidden();
        }
    }
    let c = &st.cfg;
    Json(json!({
        "server": "temoin",
        "protocol": PROTOCOL,
        "version": VERSION,
        "name": c.name,
        "recipient": c.recipient,
        "fingerprint": seal::fingerprint(&c.recipient),
        "accepting": st.accepting.load(Ordering::Relaxed),
        "auth_required": true,
        "enrollment": "invite",
        "limits": {
            "max_object_bytes": c.max_object_bytes,
            "max_video_height": c.max_video_height,
            "default_video_height": c.default_video_height.min(c.max_video_height),
            "video_bitrate_720": c.video_bitrate_720,
            "video_bitrate_1080": c.video_bitrate_1080,
            "photo_max_pixels": c.photo_max_pixels,
            "device_quota_bytes": c.device_quota_bytes,
        }
    }))
    .into_response()
}

fn valid_session(s: &str) -> bool {
    (8..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

fn token_ok(expected: &Option<String>, headers: &HeaderMap) -> bool {
    match expected {
        None => true,
        Some(t) => bearer(headers).is_some_and(|got| got.as_bytes().ct_eq(t.as_bytes()).into()),
    }
}

fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .unwrap()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap()
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// Checks the device signature of a request (see PROTOCOL.md section 4).
fn authenticate(
    st: &AppState,
    headers: &HeaderMap,
    method: &str,
    path: &str,
    body_sha256: Option<&str>,
) -> Result<crate::devices::Device, ApiError> {
    use StatusCode as S;
    if st.cfg.stealth && header(headers, "x-sr-device").is_none() {
        return Err(ApiError::new(S::NOT_FOUND, "hidden", ""));
    }
    st.registry
        .authenticate(
            header(headers, "x-sr-device"),
            header(headers, "x-sr-timestamp"),
            header(headers, "x-sr-signature"),
            method,
            path,
            body_sha256,
        )
        .map_err(|e| match e {
            AuthError::Banned => ApiError::new(S::FORBIDDEN, "banned", "this device has been banned by the server operator"),
            AuthError::UnknownDevice => ApiError::new(S::UNAUTHORIZED, "unknown_device", "device not registered on this server"),
            AuthError::Skew => ApiError::new(S::UNAUTHORIZED, "clock_skew", "phone clock too far from server time"),
            AuthError::Missing | AuthError::BadSignature => ApiError::new(S::UNAUTHORIZED, "unauthorized", e.to_string()),
        })
}

async fn put_object(
    State(st): State<Arc<AppState>>,
    Path((session, seq)): Path<(String, String)>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    use StatusCode as S;
    if st.cfg.stealth && header(&headers, "x-sr-device").is_none() {
        return Err(ApiError::new(S::NOT_FOUND, "hidden", ""));
    }
    if !st.accepting.load(Ordering::Relaxed) {
        return Err(ApiError::new(S::SERVICE_UNAVAILABLE, "paused", "server is not accepting uploads right now"));
    }
    let seq: u32 = match seq.parse() {
        Ok(n) if n <= 999_999 => n,
        _ => return Err(ApiError::new(S::BAD_REQUEST, "bad_path", "seq must be 0..=999999")),
    };
    if !valid_session(&session) {
        return Err(ApiError::new(S::BAD_REQUEST, "bad_path", "invalid session id"));
    }
    let size = body.len() as u64;
    if size > st.cfg.max_object_bytes {
        return Err(ApiError::new(S::PAYLOAD_TOO_LARGE, "too_large", "object too large"));
    }
    let sha = hex::encode(Sha256::digest(&body));
    let claimed = headers
        .get("x-sr-content-sha256")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_ascii_lowercase());
    if claimed.as_deref() != Some(sha.as_str()) {
        return Err(ApiError::new(S::BAD_REQUEST, "hash_mismatch", "X-SR-Content-SHA256 missing or does not match body"));
    }
    let dev = authenticate(&st, &headers, "PUT", uri.path(), Some(&sha))?;
    let device = dev.id.clone();
    if let Err(e) = ageformat::validate(&body) {
        return Err(ApiError::new(S::BAD_REQUEST, "bad_format", e.to_string()));
    }

    let final_path = st
        .objects_dir
        .join(&session[..2])
        .join(&session)
        .join(format!("{seq:06}.age"));

    // Idempotent retry: same bytes already stored (possibly already handed back).
    let returned_path = st.returned.lock().unwrap().path_of(&session, seq);
    let existing = match tokio::fs::read(&final_path).await {
        Ok(b) => Ok(b),
        Err(_) => tokio::fs::read(&returned_path).await,
    };
    if let Ok(existing) = existing {
        return if hex::encode(Sha256::digest(&existing)) == sha {
            let mut r = (S::OK, Json(json!({ "sha256": sha, "size": size, "duplicate": true }))).into_response();
            r.extensions_mut().insert(ErrorCode("duplicate"));
            Ok(r)
        } else {
            Err(ApiError::new(S::CONFLICT, "exists", "a different object is already stored at this path"))
        };
    }

    {
        let ledger = st.ledger.lock().await;
        if let Some(max) = st.cfg.max_total_bytes {
            if ledger.total_bytes + size > max {
                return Err(ApiError::new(S::SERVICE_UNAVAILABLE, "full", "server storage cap reached"));
            }
        }
        let (used, _) = st.device_usage(&ledger, &device);
        if used + size > dev.quota_bytes {
            return Err(ApiError::new(S::TOO_MANY_REQUESTS, "quota", "device quota exceeded"));
        }
    }
    if let Some(free) = st.free_bytes() {
        if free < st.cfg.min_free_bytes + size {
            return Err(ApiError::new(S::SERVICE_UNAVAILABLE, "full", "server disk reserve reached"));
        }
    }

    // Write + fsync to a temp file outside of the ledger lock.
    let tmp = st.tmp_dir.join(format!("{}.part", hex::encode(rand::random::<[u8; 12]>())));
    {
        let tmp = tmp.clone();
        let body = body.clone();
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&body)?;
            f.sync_all()
        })
        .await
        .map_err(|e| anyhow::anyhow!(e))?
        .map_err(|e| anyhow::anyhow!("writing temp file: {e}"))?;
    }

    let mut ledger = st.ledger.lock().await;
    let dir = final_path.parent().unwrap().to_owned();
    let link = {
        let tmp = tmp.clone();
        let final_path = final_path.clone();
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            std::fs::create_dir_all(&dir)?;
            // hard_link never overwrites: write-once even under a race.
            let r = std::fs::hard_link(&tmp, &final_path);
            let _ = std::fs::remove_file(&tmp);
            r
        })
        .await
        .map_err(|e| anyhow::anyhow!(e))?
    };
    if let Err(e) = link {
        drop(ledger);
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            // Lost a race against a concurrent upload of the same path: identical bytes
            // (typical client retry) is a success, anything else a conflict.
            let same = tokio::fs::read(&final_path)
                .await
                .is_ok_and(|existing| hex::encode(Sha256::digest(&existing)) == sha);
            if same {
                let mut r = (S::OK, Json(json!({ "sha256": sha, "size": size, "duplicate": true }))).into_response();
                r.extensions_mut().insert(ErrorCode("duplicate"));
                return Ok(r);
            }
            return Err(ApiError::new(S::CONFLICT, "exists", "object already stored at this path"));
        }
        return Err(anyhow::anyhow!("linking object: {e}").into());
    }
    let entry = ledger.append(now_rfc3339(), &device, &session, seq, size, &sha)?;
    drop(ledger);
    tracing::info!(index = entry.index, size, "object stored");

    let mut r = (
        S::CREATED,
        Json(json!({
            "sha256": entry.sha256,
            "size": entry.size,
            "received_at": entry.received_at,
            "index": entry.index,
            "chain": entry.chain,
        })),
    )
        .into_response();
    r.extensions_mut().insert(StoredIndex(entry.index));
    Ok(r)
}

// ---------------------------------------------------------------- admin

fn admin_guard(st: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    if st.cfg.stealth && !(st.cfg.admin_token.is_some() && token_ok(&st.cfg.admin_token, headers)) {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "hidden", ""));
    }
    match &st.cfg.admin_token {
        None => Err(ApiError::new(StatusCode::FORBIDDEN, "disabled", "admin API disabled (SR_ADMIN_TOKEN unset)")),
        Some(_) if token_ok(&st.cfg.admin_token, headers) => Ok(()),
        Some(_) => Err(ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized", "invalid admin token")),
    }
}

async fn admin_stats(State(st): State<Arc<AppState>>, headers: HeaderMap) -> Result<Json<serde_json::Value>, ApiError> {
    admin_guard(&st, &headers)?;
    let l = st.ledger.lock().await;
    Ok(Json(json!({
        "accepting": st.accepting.load(Ordering::Relaxed),
        "objects": l.next_index,
        "sessions": l.sessions,
        "bytes": l.total_bytes,
        "devices": l.device_bytes.len(),
        "last_received_at": l.last_received_at,
        "chain_head": l.head,
        "free_bytes": st.free_bytes(),
        "min_free_bytes": st.cfg.min_free_bytes,
        "max_total_bytes": st.cfg.max_total_bytes,
        "uptime_s": st.started.elapsed().as_secs(),
        "name": st.cfg.name,
        "version": VERSION,
        "public_url": st.public_url(),
        "tunnel": st.tunnel(),
    })))
}

#[derive(Deserialize)]
struct AcceptingBody {
    accepting: bool,
}

async fn admin_accepting(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<AcceptingBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    admin_guard(&st, &headers)?;
    st.accepting.store(body.accepting, Ordering::Relaxed);
    let persisted = serde_json::to_vec(&PersistedState { accepting: Some(body.accepting) }).unwrap();
    tokio::fs::write(&st.state_file, persisted).await.map_err(anyhow::Error::from)?;
    tracing::info!(accepting = body.accepting, "intake toggled");
    Ok(Json(json!({ "accepting": body.accepting })))
}

/// Same as [`admin_accepting`] but tags the journal entry with the new state.
async fn admin_accepting_logged(
    st: State<Arc<AppState>>,
    headers: HeaderMap,
    body: Json<AcceptingBody>,
) -> Response {
    let new = body.accepting;
    match admin_accepting(st, headers, body).await {
        Ok(j) => {
            let mut r = j.into_response();
            r.extensions_mut().insert(Note(format!("accepting={new}")));
            r
        }
        Err(e) => e.into_response(),
    }
}

fn qr_svg(data: &str) -> Result<String, ApiError> {
    Ok(qrcode::QrCode::with_error_correction_level(data.as_bytes(), qrcode::EcLevel::M)
        .map_err(|e| anyhow::anyhow!("qr: {e}"))?
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(320, 320)
        .quiet_zone(true)
        .build())
}

fn with_note<T: IntoResponse>(r: T, note: String) -> Response {
    let mut r = r.into_response();
    r.extensions_mut().insert(Note(note));
    r
}

// ---------------------------------------------------------------- devices

#[derive(Deserialize)]
struct RegisterBody {
    invite: String,
    public_key: String,
    timestamp: i64,
    signature: String,
}

/// A phone joins with an invitation code and its own public key.
async fn register_device(State(st): State<Arc<AppState>>, Json(b): Json<RegisterBody>) -> Result<Response, ApiError> {
    use StatusCode as S;
    let dev = st.registry.register(&b.invite, &b.public_key, b.timestamp, &b.signature).map_err(|e| {
        let (code, err) = match e {
            RegisterError::UnknownInvite => (S::NOT_FOUND, "unknown_invite"),
            RegisterError::Revoked => (S::GONE, "invite_revoked"),
            RegisterError::Expired => (S::GONE, "invite_expired"),
            RegisterError::Exhausted => (S::GONE, "invite_exhausted"),
            RegisterError::Banned => (S::FORBIDDEN, "banned"),
            RegisterError::BadKey | RegisterError::BadSignature => (S::BAD_REQUEST, "bad_request"),
        };
        ApiError::new(code, err, e.to_string())
    })?;
    let note = format!("registered {} via invite {}", dev.name, dev.invite_id);
    Ok(with_note(
        Json(json!({ "device_id": dev.id, "name": dev.name, "quota_bytes": dev.quota_bytes })),
        note,
    ))
}

/// What a device may know about itself: counters only, never the list of its objects.
async fn me(State(st): State<Arc<AppState>>, uri: Uri, headers: HeaderMap) -> Result<Json<serde_json::Value>, ApiError> {
    let dev = authenticate(&st, &headers, "GET", uri.path(), None)?;
    let l = st.ledger.lock().await;
    let (used, objects) = st.device_usage(&l, &dev.id);
    Ok(Json(json!({
        "device_id": dev.id,
        "name": dev.name,
        "quota_bytes": dev.quota_bytes,
        "used_bytes": used,
        "objects": objects,
        "last_upload_at": l.device_last.get(&dev.id),
        "accepting": st.accepting.load(Ordering::Relaxed),
        "request": st.shares.device_status(&dev.id).map(|(r, state)| request_json(&r, state)),
    })))
}

#[derive(Deserialize)]
struct InviteBody {
    #[serde(default)]
    label: String,
    expires_hours: Option<u32>,
    max_uses: Option<u32>,
    quota_gb: Option<f64>,
}

fn invite_json(st: &AppState, i: &crate::devices::Invite) -> serde_json::Value {
    let expired = i
        .expires_at
        .as_deref()
        .and_then(|e| time::OffsetDateTime::parse(e, &time::format_description::well_known::Rfc3339).ok())
        .is_some_and(|e| time::OffsetDateTime::now_utc() > e);
    let exhausted = i.max_uses.is_some_and(|m| i.uses >= m);
    json!({
        "id": i.id, "code": i.code, "label": i.label, "created_at": i.created_at,
        "expires_at": i.expires_at, "max_uses": i.max_uses, "uses": i.uses,
        "quota_bytes": i.quota_bytes, "revoked": i.revoked,
        "active": !i.revoked && !expired && !exhausted,
        "link": st.enroll_link(&i.code),
    })
}

async fn admin_invites(State(st): State<Arc<AppState>>, headers: HeaderMap) -> Result<Json<serde_json::Value>, ApiError> {
    admin_guard(&st, &headers)?;
    let mut list: Vec<_> = st.registry.invites().iter().map(|i| invite_json(&st, i)).collect();
    list.reverse();
    Ok(Json(json!({ "invites": list, "fingerprint": seal::fingerprint(&st.cfg.recipient) })))
}

#[derive(Deserialize)]
struct QrQuery {
    id: String,
}

async fn admin_create_invite(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(b): Json<InviteBody>,
) -> Result<Response, ApiError> {
    admin_guard(&st, &headers)?;
    let quota = b
        .quota_gb
        .filter(|q| q.is_finite() && *q > 0.0)
        .map(|q| (q * 1024.0 * 1024.0 * 1024.0) as u64)
        .unwrap_or(st.cfg.device_quota_bytes);
    let inv = st.registry.create_invite(NewInvite {
        label: if b.label.trim().is_empty() { "Invitation".into() } else { b.label },
        expires_hours: b.expires_hours.filter(|h| *h > 0),
        max_uses: b.max_uses.filter(|m| *m > 0),
        quota_bytes: quota,
    })?;
    let mut j = invite_json(&st, &inv);
    j["svg"] = json!(qr_svg(&st.enroll_link(&inv.code))?);
    let note = format!("invite {} created ({})", inv.id, inv.label);
    Ok(with_note(Json(j), note))
}

/// QR code of an existing invitation (the admin page shows it on demand).
async fn admin_invite_qr(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<QrQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    admin_guard(&st, &headers)?;
    let inv = st
        .registry
        .invites()
        .into_iter()
        .find(|i| i.id == q.id)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "not_found", "no such invitation"))?;
    let link = st.enroll_link(&inv.code);
    Ok(Json(json!({ "svg": qr_svg(&link)?, "link": link })))
}

async fn admin_revoke_invite(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    admin_guard(&st, &headers)?;
    if !st.registry.revoke_invite(&id)? {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "not_found", "no such invitation"));
    }
    Ok(with_note(Json(json!({ "revoked": true })), format!("invite {id} revoked")))
}

async fn admin_devices(State(st): State<Arc<AppState>>, headers: HeaderMap) -> Result<Json<serde_json::Value>, ApiError> {
    admin_guard(&st, &headers)?;
    let invites = st.registry.invites();
    let l = st.ledger.lock().await;
    let mut list: Vec<_> = st
        .registry
        .devices()
        .iter()
        .map(|d| {
            json!({
                "id": d.id, "name": d.name, "created_at": d.created_at,
                "invite": invites.iter().find(|i| i.id == d.invite_id).map(|i| i.label.clone()).unwrap_or_default(),
                "quota_bytes": d.quota_bytes, "banned": d.banned, "banned_at": d.banned_at,
                "used_bytes": st.device_usage(&l, &d.id).0,
                "objects": st.device_usage(&l, &d.id).1,
                "returned_bytes": st.returned.lock().unwrap().device_bytes(&d.id),
                "last_upload_at": l.device_last.get(&d.id),
            })
        })
        .collect();
    list.reverse();
    Ok(Json(json!({ "devices": list })))
}

#[derive(Deserialize)]
struct DeviceUpdate {
    banned: Option<bool>,
    name: Option<String>,
}

async fn admin_update_device(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(b): Json<DeviceUpdate>,
) -> Result<Response, ApiError> {
    admin_guard(&st, &headers)?;
    let mut notes = Vec::new();
    if let Some(banned) = b.banned {
        if !st.registry.set_banned(&id, banned)? {
            return Err(ApiError::new(StatusCode::NOT_FOUND, "not_found", "no such device"));
        }
        notes.push(if banned { format!("device {id} banned") } else { format!("device {id} unbanned") });
    }
    if let Some(name) = b.name.filter(|n| !n.trim().is_empty()) {
        if !st.registry.rename(&id, &name)? {
            return Err(ApiError::new(StatusCode::NOT_FOUND, "not_found", "no such device"));
        }
        notes.push(format!("device {id} renamed"));
    }
    Ok(with_note(Json(json!({ "ok": true })), notes.join("; ")))
}


#[derive(Deserialize)]
struct EventsQuery {
    limit: Option<usize>,
}

async fn admin_events(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<EventsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    admin_guard(&st, &headers)?;
    let limit = q.limit.unwrap_or(200).min(1000);
    Ok(Json(json!({ "events": st.events.recent(limit) })))
}

// ---------------------------------------------------------------- hand-back

fn share_error(e: ShareError) -> ApiError {
    use StatusCode as S;
    let (code, err) = match e {
        // Unknown link: indistinguishable from any other unknown path.
        ShareError::NotFound => return ApiError::new(S::NOT_FOUND, "hidden", ""),
        ShareError::NotReady => (S::CONFLICT, "not_ready"),
        ShareError::Used => (S::GONE, "used"),
        ShareError::Done => (S::GONE, "done"),
        ShareError::Expired => (S::GONE, "expired"),
        ShareError::Revoked => (S::GONE, "revoked"),
        ShareError::BadOffset(_) => (S::CONFLICT, "bad_offset"),
        ShareError::TooLarge => (S::PAYLOAD_TOO_LARGE, "too_large"),
        ShareError::Incomplete => (S::CONFLICT, "incomplete"),
        ShareError::BadFile => (S::NOT_FOUND, "bad_file"),
        ShareError::Invalid(_) => (S::BAD_REQUEST, "bad_request"),
        ShareError::Io(ref m) => {
            tracing::error!("share storage: {m}");
            return ApiError::new(S::INTERNAL_SERVER_ERROR, "internal", "internal error");
        }
    };
    ApiError::new(code, err, e.to_string())
}

fn state_str(s: shares::State) -> &'static str {
    match s {
        shares::State::Uploading => "uploading",
        shares::State::Ready => "ready",
        shares::State::Opened => "opened",
        shares::State::Done => "done",
        shares::State::Expired => "expired",
        shares::State::Revoked => "revoked",
    }
}

/// What the phone sees about its request: the code to pass on and where the link stands.
fn request_json(r: &shares::HandRequest, link: Option<shares::State>) -> serde_json::Value {
    json!({
        "code": shares::display_code(&r.code),
        "created_at": r.created_at,
        "expires_at": r.expires_at,
        "link": link.map(state_str),
    })
}

/// The phone asks for its recordings back: returns its open request code (or a new one).
async fn me_request(State(st): State<Arc<AppState>>, uri: Uri, headers: HeaderMap) -> Result<Response, ApiError> {
    let dev = authenticate(&st, &headers, "POST", uri.path(), None)?;
    let r = st.shares.request_for_device(&dev.id).map_err(share_error)?;
    let link = st.shares.device_status(&dev.id).and_then(|(_, s)| s);
    Ok(with_note(Json(request_json(&r, link)), "hand-back requested".into()))
}

fn no_store(mut r: Response) -> Response {
    let h = r.headers_mut();
    h.insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-store"));
    h.insert(header::REFERRER_POLICY, header::HeaderValue::from_static("no-referrer"));
    r
}

/// The download page. Served only for links that exist (or existed): anything else is a
/// bare 404. The page itself is static and identical for every link.
async fn share_page(State(st): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    if st.shares.get(&id).is_none() {
        return hidden();
    }
    let mut r = no_store(Html(SHARE_HTML).into_response());
    let h = r.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        header::HeaderValue::from_static(
            "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; \
             img-src blob: data:; media-src blob:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
        ),
    );
    h.insert(header::X_FRAME_OPTIONS, header::HeaderValue::from_static("DENY"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, header::HeaderValue::from_static("nosniff"));
    r
}

/// What the page shows before opening: number of files, size, validity. Never names.
async fn share_meta(State(st): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Response, ApiError> {
    let sh = st.shares.get(&id).ok_or_else(|| share_error(ShareError::NotFound))?;
    Ok(no_store(
        Json(json!({
            "state": state_str(sh.state),
            "files": sh.files.len().saturating_sub(1),
            "bytes": sh.total_bytes(),
            "expires_at": sh.expires_at,
            "window_secs": shares::CLAIM_WINDOW_SECS,
            "window_end": sh.window_end(),
        }))
        .into_response(),
    ))
}

/// Opens the link. First call: claim token (from then on only this browser may download).
async fn share_open(State(st): State<Arc<AppState>>, Path(id): Path<String>, headers: HeaderMap) -> Result<Response, ApiError> {
    let presented = header(&headers, "x-sr-claim");
    let (token, sh) = st.shares.claim(&id, presented).map_err(share_error)?;
    let note = if presented.is_some() { "share resumed" } else { "share opened" };
    Ok(with_note(
        no_store(Json(json!({ "claim": token, "files": sh.files, "until": sh.window_end() })).into_response()),
        note.into(),
    ))
}

async fn share_file(
    State(st): State<Arc<AppState>>,
    Path((id, n)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let n: usize = n.parse().map_err(|_| share_error(ShareError::BadFile))?;
    let (path, size) = st.shares.file(&id, n, header(&headers, "x-sr-claim")).map_err(share_error)?;
    let f = tokio::fs::File::open(&path).await.map_err(|e| share_error(e.into()))?;
    let body = axum::body::Body::from_stream(tokio_util::io::ReaderStream::with_capacity(f, 256 * 1024));
    let r = Response::builder()
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, size)
        .body(body)
        .map_err(|e| anyhow::anyhow!(e))?;
    Ok(no_store(r))
}

async fn share_done(State(st): State<Arc<AppState>>, Path(id): Path<String>, headers: HeaderMap) -> Result<Response, ApiError> {
    let token = header(&headers, "x-sr-claim").ok_or_else(|| share_error(ShareError::Used))?;
    st.shares.finish(&id, token).map_err(share_error)?;
    let st2 = st.clone();
    let _ = tokio::task::spawn_blocking(move || st2.process_returns()).await;
    Ok(with_note(Json(json!({ "done": true })), "share retrieved by recipient".into()))
}

fn share_json(sh: &shares::Share) -> serde_json::Value {
    json!({
        "id": sh.id, "state": state_str(sh.state), "label": sh.label,
        "created_at": sh.created_at, "expires_at": sh.expires_at,
        "opened_at": sh.opened_at, "closed_at": sh.closed_at, "window_end": sh.window_end(),
        "files": sh.files.len().saturating_sub(1), "bytes": sh.total_bytes(),
        "request": sh.request_code.as_deref().map(shares::display_code),
        "returned": sh.returned,
    })
}

fn admin_request_json(st: &AppState, l: &Ledger, r: &shares::HandRequest, all: &[shares::Share]) -> serde_json::Value {
    let dev = st.registry.device(&r.device_id);
    let share = r.share_id.as_ref().and_then(|id| all.iter().find(|s| &s.id == id));
    json!({
        "code": shares::display_code(&r.code),
        "device_id": r.device_id,
        "device_name": dev.as_ref().map(|d| d.name.clone()),
        "banned": dev.as_ref().is_some_and(|d| d.banned),
        "created_at": r.created_at, "expires_at": r.expires_at, "cancelled": r.cancelled,
        "objects": st.device_usage(l, &r.device_id).1,
        "bytes": st.device_usage(l, &r.device_id).0,
        "returned_objects": st.returned.lock().unwrap().device_objects(&r.device_id),
        "returned_bytes": st.returned.lock().unwrap().device_bytes(&r.device_id),
        "share": share.map(share_json),
    })
}

async fn admin_requests(State(st): State<Arc<AppState>>, headers: HeaderMap) -> Result<Json<serde_json::Value>, ApiError> {
    admin_guard(&st, &headers)?;
    let all = st.shares.shares();
    let l = st.ledger.lock().await;
    let requests: Vec<_> = st.shares.requests().iter().rev().map(|r| admin_request_json(&st, &l, r, &all)).collect();
    let orphans: Vec<_> = all.iter().rev().filter(|s| s.request_code.is_none()).map(share_json).collect();
    Ok(Json(json!({ "requests": requests, "shares": orphans })))
}

/// Looks a request code up (used by `sr share`).
async fn admin_request(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(code): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    admin_guard(&st, &headers)?;
    let r = st.shares.request(&code).ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "not_found", "unknown request code"))?;
    let all = st.shares.shares();
    let l = st.ledger.lock().await;
    Ok(Json(admin_request_json(&st, &l, &r, &all)))
}

/// The encrypted objects of the device behind a hand-back request, for an operator who
/// decrypts them in the browser: the private key never reaches the server, which only
/// serves ciphertext it cannot read. Built from the verified ledger.
async fn admin_request_objects(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(code): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    admin_guard(&st, &headers)?;
    let r = st.shares.request(&code).ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "not_found", "unknown request code"))?;
    let path = st.cfg.data_dir.join("ledger.jsonl");
    let entries = tokio::task::spawn_blocking(move || crate::ledger::verify_file(&path))
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "ledger", format!("{e:#}")))?;
    let mine: Vec<_> = entries.iter().filter(|e| e.device == r.device_id).collect();
    Ok(Json(json!({
        "code": shares::display_code(&r.code),
        "device_id": r.device_id,
        "cancelled": r.cancelled,
        "upto_index": mine.iter().map(|e| e.index).max(),
        "objects": mine.iter().map(|e| json!({ "index": e.index, "session": e.session, "seq": e.seq, "size": e.size })).collect::<Vec<_>>(),
    })))
}

/// One stored object, still encrypted (from `objects/` or, once handed back, `returned/`).
async fn admin_object(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((session, seq)): Path<(String, u32)>,
) -> Result<Response, ApiError> {
    admin_guard(&st, &headers)?;
    if !valid_session(&session) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "bad_session", "invalid session"));
    }
    let stored = st.objects_dir.join(&session[..2]).join(&session).join(format!("{seq:06}.age"));
    let returned = st.returned.lock().unwrap().path_of(&session, seq);
    let bytes = match tokio::fs::read(&stored).await {
        Ok(b) => b,
        Err(_) => tokio::fs::read(&returned)
            .await
            .map_err(|_| ApiError::new(StatusCode::NOT_FOUND, "not_found", "no such object"))?,
    };
    let mut r = bytes.into_response();
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, header::HeaderValue::from_static("application/octet-stream"));
    h.insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-store"));
    Ok(r)
}

async fn admin_cancel_request(State(st): State<Arc<AppState>>, headers: HeaderMap, Path(code): Path<String>) -> Result<Response, ApiError> {
    admin_guard(&st, &headers)?;
    if !st.shares.cancel_request(&code).map_err(share_error)? {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "not_found", "unknown request code"));
    }
    Ok(with_note(Json(json!({ "cancelled": true })), "hand-back request cancelled".into()))
}

#[derive(Deserialize)]
struct ShareBody {
    code: Option<String>,
    /// Highest ledger index of the device's objects included (sent by `sr share`).
    upto_index: Option<u64>,
    #[serde(default)]
    label: String,
    files: Vec<u64>,
    days: Option<u32>,
}

async fn admin_create_share(State(st): State<Arc<AppState>>, headers: HeaderMap, Json(b): Json<ShareBody>) -> Result<Response, ApiError> {
    admin_guard(&st, &headers)?;
    let total: u64 = b.files.iter().sum();
    if let Some(free) = st.free_bytes() {
        if free < st.cfg.min_free_bytes + total {
            return Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "full", "server disk reserve reached"));
        }
    }
    // Older `sr` does not say what it included: everything received so far.
    let upto_index = match b.upto_index {
        Some(i) => Some(i),
        None => st.ledger.lock().await.next_index.checked_sub(1),
    };
    let sh = st
        .shares
        .create(NewShare {
            request_code: b.code.filter(|c| !c.trim().is_empty()),
            upto_index,
            label: b.label,
            files: b.files,
            days: b.days.unwrap_or(shares::DEFAULT_SHARE_DAYS),
        })
        .map_err(share_error)?;
    let base = st.public_url();
    let note = format!("share created ({} files, {} bytes)", sh.files.len().saturating_sub(1), total);
    Ok(with_note(
        Json(json!({ "id": sh.id, "expires_at": sh.expires_at, "link": format!("{base}/r/{}", sh.id) })),
        note,
    ))
}

#[derive(Deserialize)]
struct OffsetQuery {
    offset: u64,
}

async fn admin_share_part(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((id, n)): Path<(String, usize)>,
    Query(q): Query<OffsetQuery>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, ApiError> {
    admin_guard(&st, &headers)?;
    let st2 = st.clone();
    let len = tokio::task::spawn_blocking(move || st2.shares.append(&id, n, q.offset, &body))
        .await
        .map_err(|e| anyhow::anyhow!(e))?
        .map_err(share_error)?;
    Ok(Json(json!({ "length": len })))
}

async fn admin_seal_share(State(st): State<Arc<AppState>>, headers: HeaderMap, Path(id): Path<String>) -> Result<Response, ApiError> {
    admin_guard(&st, &headers)?;
    let sh = st.shares.seal(&id).map_err(share_error)?;
    Ok(with_note(Json(share_json(&sh)), "share ready".into()))
}

async fn admin_revoke_share(State(st): State<Arc<AppState>>, headers: HeaderMap, Path(id): Path<String>) -> Result<Response, ApiError> {
    admin_guard(&st, &headers)?;
    if !st.shares.revoke(&id).map_err(share_error)? {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "not_found", "no such link"));
    }
    Ok(with_note(Json(json!({ "revoked": true })), "share revoked".into()))
}

/// In stealth mode the login page is neutral: no server name, no product name.
/// The page picks its language (English, or French for French browsers) on its own.
async fn admin_page(State(st): State<Arc<AppState>>) -> Html<String> {
    let name = if st.cfg.stealth { String::new() } else { html_escape(&st.cfg.name) };
    Html(ADMIN_HTML.replace("{{STEALTH}}", if st.cfg.stealth { "true" } else { "false" }).replace("{{NAME}}", &name))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

const SHARE_HTML: &str = include_str!("../web/share.html");
const LANDING_HTML: &str = include_str!("../web/landing.html");
const ADMIN_HTML: &str = include_str!("../web/admin.html");
/// Browser hand-back (built from tools/handback-web), loaded by the admin page on demand.
const HANDBACK_JS: &str = include_str!("../web/handback.js");

async fn admin_handback_js(State(st): State<Arc<AppState>>, headers: HeaderMap) -> Result<Response, ApiError> {
    admin_guard(&st, &headers)?;
    let mut r = HANDBACK_JS.into_response();
    r.headers_mut().insert(header::CONTENT_TYPE, header::HeaderValue::from_static("text/javascript; charset=utf-8"));
    Ok(r)
}
