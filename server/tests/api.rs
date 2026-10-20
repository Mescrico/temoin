use std::io::Read;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use http_body_util::BodyExt;
use sha2::{Digest, Sha256};
use srec::config::Config;
use srec::devices::{register_message, request_message};
use srec::envelope::{self, Header};
use srec::server::{router, AppState};
use tower::ServiceExt;

const ADMIN: &str = "Bearer admin-secret";

fn header(session: &str, seq: u32) -> Header {
    Header {
        v: 1,
        session: session.into(),
        seq,
        kind: "video".into(),
        container: "mpegts".into(),
        is_final: false,
        created_at: "2026-10-19T12:00:00Z".into(),
        start_ms: Some(0),
        duration_ms: Some(4000),
        device: None,
        location: None,
        video: None,
    }
}

fn decrypt(id: &age::x25519::Identity, bytes: &[u8]) -> Vec<u8> {
    let d = age::Decryptor::new(bytes).unwrap();
    let mut r = d.decrypt(std::iter::once(id as &dyn age::Identity)).unwrap();
    let mut out = Vec::new();
    r.read_to_end(&mut out).unwrap();
    out
}

fn now() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

/// A phone: its own Ed25519 key, and once registered, its device id.
struct Phone {
    key: SigningKey,
    id: String,
}

impl Phone {
    fn new() -> Self {
        let key = SigningKey::generate(&mut rand::rngs::OsRng);
        let id = srec::devices::device_id(key.verifying_key().as_bytes());
        Phone { key, id }
    }

    fn pubkey(&self) -> String {
        STANDARD.encode(self.key.verifying_key().as_bytes())
    }

    fn register_req(&self, invite: &str) -> Request<Body> {
        let ts = now();
        let sig = self.key.sign(register_message(invite, &self.pubkey(), ts).as_bytes());
        let body = serde_json::json!({
            "invite": invite, "public_key": self.pubkey(), "timestamp": ts,
            "signature": STANDARD.encode(sig.to_bytes()),
        });
        Request::post("/api/v1/devices")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    fn signed(&self, method: &str, path: &str, body: Vec<u8>, ts: i64) -> Request<Body> {
        let sha = (!body.is_empty() || method == "PUT").then(|| hex::encode(Sha256::digest(&body)));
        let sig = self.key.sign(request_message(method, path, sha.as_deref(), ts, &self.id).as_bytes());
        let mut b = Request::builder()
            .method(method)
            .uri(path)
            .header("x-sr-device", &self.id)
            .header("x-sr-timestamp", ts.to_string())
            .header("x-sr-signature", STANDARD.encode(sig.to_bytes()));
        if let Some(sha) = sha {
            b = b.header("x-sr-content-sha256", sha);
        }
        b.body(Body::from(body)).unwrap()
    }

    fn put(&self, session: &str, seq: u32, body: Vec<u8>) -> Request<Body> {
        self.signed("PUT", &format!("/api/v1/objects/{session}/{seq}"), body, now())
    }
}

async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null))
}

fn admin(method: &str, path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", ADMIN)
        .header("content-type", "application/json")
        .header("x-real-ip", "203.0.113.7")
        .body(if body.is_null() { Body::empty() } else { Body::from(body.to_string()) })
        .unwrap()
}

async fn invite(app: &axum::Router, body: serde_json::Value) -> serde_json::Value {
    let (s, inv) = send(app, admin("POST", "/api/v1/admin/invites", body)).await;
    assert_eq!(s, StatusCode::OK, "{inv}");
    inv
}

fn setup() -> (tempfile::TempDir, std::sync::Arc<AppState>, axum::Router, age::x25519::Identity, [u8; 32]) {
    let dir = tempfile::tempdir().unwrap();
    let id = age::x25519::Identity::generate();
    let recipient = id.to_public().to_string();
    let key = srec::seal::parse_recipient(&recipient).unwrap();
    let state = AppState::open(Config::for_tests(dir.path().to_owned(), recipient)).unwrap();
    let app = router(state.clone());
    (dir, state, app, id, key)
}

#[test]
fn reference_sealer_roundtrips_with_age_crate() {
    let id = age::x25519::Identity::generate();
    let key = srec::seal::parse_recipient(&id.to_public().to_string()).unwrap();
    for len in [1usize, 100, 65535, 65536, 65537, 3 * 65536, 3 * 65536 + 7, 1_000_000] {
        let pt: Vec<u8> = (0..len).map(|i| (i * 31 % 251) as u8).collect();
        let ct = srec::seal::seal(&key, &pt);
        srec::ageformat::validate(&ct).unwrap_or_else(|e| panic!("len {len}: {e}"));
        assert_eq!(decrypt(&id, &ct), pt, "len {len}");
    }
}

#[tokio::test]
async fn upload_flow() {
    let (dir, state, app, id, key) = setup();

    let (s, info) = send(&app, Request::get("/api/v1/info").header("x-real-ip", "203.0.113.7").body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(info["enrollment"], "invite");

    // invitation -> registration
    let inv = invite(&app, serde_json::json!({ "label": "Manif test", "max_uses": 5, "quota_gb": 0.01 })).await;
    let code = inv["code"].as_str().unwrap().to_string();
    assert!(inv["link"].as_str().unwrap().contains(&format!("&i={code}")));
    assert!(inv["svg"].as_str().unwrap().starts_with("<?xml"));
    let phone = Phone::new();
    let (s, reg) = send(&app, phone.register_req(&code)).await;
    assert_eq!(s, StatusCode::OK, "{reg}");
    assert_eq!(reg["device_id"], phone.id);
    assert_eq!(reg["name"], "Phone #1");
    // scanning again = same identity, invitation not consumed twice
    let (s, again) = send(&app, phone.register_req(&code)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(again["device_id"], phone.id);

    let session = "sess_ABCDEFGH1234";
    let pt = envelope::encode(&header(session, 0), &vec![7u8; 200_000]);
    let obj = srec::seal::seal(&key, &pt);
    let path = format!("/api/v1/objects/{session}/0");
    let sha = hex::encode(Sha256::digest(&obj));

    // unsigned
    let unsigned = Request::put(&path).header("x-sr-content-sha256", &sha).body(Body::from(obj.clone())).unwrap();
    assert_eq!(send(&app, unsigned).await.0, StatusCode::UNAUTHORIZED);
    // signed by an unregistered key
    let (s, e) = send(&app, Phone::new().put(session, 0, obj.clone())).await;
    assert_eq!((s, e["error"].as_str()), (StatusCode::UNAUTHORIZED, Some("unknown_device")));
    // signature for another path cannot be replayed here
    let mut wrong = phone.put(session, 1, obj.clone());
    *wrong.uri_mut() = path.parse().unwrap();
    assert_eq!(send(&app, wrong).await.0, StatusCode::UNAUTHORIZED);
    // clock far off
    let (s, e) = send(&app, phone.signed("PUT", &path, obj.clone(), now() - 2 * 86400)).await;
    assert_eq!((s, e["error"].as_str()), (StatusCode::UNAUTHORIZED, Some("clock_skew")));
    // plaintext refused
    let (s, e) = send(&app, phone.put(session, 0, b"\xff\xd8\xff not encrypted".to_vec())).await;
    assert_eq!((s, e["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_format")));
    // truncated on a chunk boundary
    let hdr = srec::ageformat::validate(&obj).unwrap();
    let cut = obj[..hdr + 16 + 3 * (65536 + 16) + 10].to_vec();
    assert_eq!(send(&app, phone.put(session, 0, cut)).await.0, StatusCode::BAD_REQUEST);

    // ok, then idempotent retry, then conflict
    let (s, r) = send(&app, phone.put(session, 0, obj.clone())).await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    assert_eq!(r["sha256"], sha);
    let (s, r) = send(&app, phone.put(session, 0, obj.clone())).await;
    assert_eq!((s, r["duplicate"].as_bool()), (StatusCode::OK, Some(true)));
    assert_eq!(send(&app, phone.put(session, 0, srec::seal::seal(&key, &pt))).await.0, StatusCode::CONFLICT);

    let stored = std::fs::read(dir.path().join("objects/se").join(session).join("000000.age")).unwrap();
    let pt2 = decrypt(&id, &stored);
    assert_eq!(envelope::decode(&pt2).unwrap().0.session, session);

    // no read route
    assert_eq!(send(&app, Request::get(&path).body(Body::empty()).unwrap()).await.0, StatusCode::METHOD_NOT_ALLOWED);

    // the device sees its own counters
    let (s, me) = send(&app, phone.signed("GET", "/api/v1/me", vec![], now())).await;
    assert_eq!(s, StatusCode::OK, "{me}");
    assert_eq!(me["objects"], 1);
    assert_eq!(me["used_bytes"], obj.len());
    assert_eq!(me["name"], "Phone #1");

    // quota from the invitation (0.01 GiB ~ 10.7 MB, objects max 4 MiB)
    let big = srec::seal::seal(&key, &envelope::encode(&header(session, 1), &vec![1u8; 3_900_000]));
    for seq in 1..=2 {
        assert_eq!(send(&app, phone.put(session, seq, big.clone())).await.0, StatusCode::CREATED);
    }
    let (s, e) = send(&app, phone.put(session, 3, big.clone())).await;
    assert_eq!((s, e["error"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("quota")));

    // pause
    assert_eq!(send(&app, admin("POST", "/api/v1/admin/accepting", serde_json::json!({ "accepting": false }))).await.0, StatusCode::OK);
    let small = srec::seal::seal(&key, &envelope::encode(&header("other_session_1", 0), b"x"));
    assert_eq!(send(&app, phone.put("other_session_1", 0, small)).await.0, StatusCode::SERVICE_UNAVAILABLE);

    // admin device list carries per-device counters
    let (_, devs) = send(&app, admin("GET", "/api/v1/admin/devices", serde_json::Value::Null)).await;
    assert_eq!(devs["devices"][0]["objects"], 3);
    assert_eq!(devs["devices"][0]["invite"], "Manif test");

    // journal
    let ev = state.events.recent(1000);
    assert!(ev.iter().any(|e| e.method == "START"));
    assert!(ev.iter().any(|e| e.error.as_deref() == Some("unknown_device")));
    assert!(ev.iter().any(|e| e.error.as_deref() == Some("duplicate")));
    assert!(ev.iter().any(|e| e.error.as_deref() == Some("quota")));
    assert!(ev.iter().any(|e| e.status == 201 && e.index == Some(0) && e.device.as_deref() == Some(phone.id.as_str())));
    assert!(ev.iter().any(|e| e.note.as_deref() == Some("accepting=false")));
    assert!(ev.iter().any(|e| e.note.as_deref().is_some_and(|n| n.starts_with("registered Phone #1"))));
    assert!(ev.iter().any(|e| e.ip.as_deref() == Some("203.0.113.7")));

    assert_eq!(srec::ledger::verify_file(&dir.path().join("ledger.jsonl")).unwrap().len(), 3);
}

#[tokio::test]
async fn invitations_and_bans() {
    let (_dir, _state, app, _id, key) = setup();
    let obj = || srec::seal::seal(&key, &envelope::encode(&header("ban_session_01", 0), b"hello"));

    // unknown invitation
    assert_eq!(send(&app, Phone::new().register_req("NOPE")).await.0, StatusCode::NOT_FOUND);

    // single-use invitation
    let inv = invite(&app, serde_json::json!({ "label": "one", "max_uses": 1 })).await;
    let code = inv["code"].as_str().unwrap().to_string();
    let a = Phone::new();
    assert_eq!(send(&app, a.register_req(&code)).await.0, StatusCode::OK);
    let (s, e) = send(&app, Phone::new().register_req(&code)).await;
    assert_eq!((s, e["error"].as_str()), (StatusCode::GONE, Some("invite_exhausted")));

    // revoked invitation: new phones refused, existing ones keep working
    let inv2 = invite(&app, serde_json::json!({ "label": "two" })).await;
    let code2 = inv2["code"].as_str().unwrap().to_string();
    let b = Phone::new();
    assert_eq!(send(&app, b.register_req(&code2.to_lowercase())).await.0, StatusCode::OK, "codes are case-insensitive");
    let rid = inv2["id"].as_str().unwrap();
    assert_eq!(send(&app, admin("POST", &format!("/api/v1/admin/invites/{rid}/revoke"), serde_json::Value::Null)).await.0, StatusCode::OK);
    let (s, e) = send(&app, Phone::new().register_req(&code2)).await;
    assert_eq!((s, e["error"].as_str()), (StatusCode::GONE, Some("invite_revoked")));
    assert_eq!(send(&app, b.put("ban_session_01", 0, obj())).await.0, StatusCode::CREATED);

    // ban one device: only that one is blocked, including re-registration
    assert_eq!(send(&app, admin("POST", &format!("/api/v1/admin/devices/{}", a.id), serde_json::json!({ "banned": true }))).await.0, StatusCode::OK);
    let (s, e) = send(&app, a.put("ban_session_02", 0, obj())).await;
    assert_eq!((s, e["error"].as_str()), (StatusCode::FORBIDDEN, Some("banned")));
    assert_eq!(send(&app, a.register_req(&code)).await.0, StatusCode::FORBIDDEN);
    assert_eq!(send(&app, a.signed("GET", "/api/v1/me", vec![], now())).await.0, StatusCode::FORBIDDEN);
    assert_eq!(send(&app, b.put("ban_session_03", 0, obj())).await.0, StatusCode::CREATED);

    // unban + rename
    let upd = serde_json::json!({ "banned": false, "name": "Alice" });
    assert_eq!(send(&app, admin("POST", &format!("/api/v1/admin/devices/{}", a.id), upd)).await.0, StatusCode::OK);
    assert_eq!(send(&app, a.put("ban_session_02", 0, obj())).await.0, StatusCode::CREATED);
    let (_, me) = send(&app, a.signed("GET", "/api/v1/me", vec![], now())).await;
    assert_eq!(me["name"], "Alice");

    // admin API requires the admin token
    let anon = Request::get("/api/v1/admin/devices").body(Body::empty()).unwrap();
    assert_eq!(send(&app, anon).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn registry_survives_restart() {
    let (dir, state, app, id, _key) = setup();
    let inv = invite(&app, serde_json::json!({ "label": "persist" })).await;
    let p = Phone::new();
    assert_eq!(send(&app, p.register_req(inv["code"].as_str().unwrap())).await.0, StatusCode::OK);
    drop(app);
    drop(state);
    let state = AppState::open(Config::for_tests(dir.path().to_owned(), id.to_public().to_string())).unwrap();
    assert_eq!(state.registry.devices().len(), 1);
    let app = router(state);
    assert_eq!(send(&app, p.signed("GET", "/api/v1/me", vec![], now())).await.0, StatusCode::OK);
}

#[test]
fn ledger_detects_tampering() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.jsonl");
    {
        let mut l = srec::ledger::Ledger::open(&path).unwrap();
        for i in 0..3 {
            l.append("2026-10-19T12:00:00Z".into(), "0123456789abcdef0123456789abcdef", "sessionAAAA", i, 10, "ab").unwrap();
        }
    }
    assert_eq!(srec::ledger::verify_file(&path).unwrap().len(), 3);
    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    std::fs::write(&path, format!("{}\n{}\n", lines[0], lines[2])).unwrap();
    assert!(srec::ledger::verify_file(&path).is_err());
    std::fs::write(&path, text.replacen("\"size\":10", "\"size\":11", 1)).unwrap();
    assert!(srec::ledger::verify_file(&path).is_err());
}

#[tokio::test]
async fn stealth_mode_reveals_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let id = age::x25519::Identity::generate();
    let key = srec::seal::parse_recipient(&id.to_public().to_string()).unwrap();
    let mut cfg = Config::for_tests(dir.path().to_owned(), id.to_public().to_string());
    cfg.stealth = true;
    let app = router(AppState::open(cfg).unwrap());

    let get = |p: &str| Request::get(p).body(Body::empty()).unwrap();
    let raw = |r: Request<Body>| {
        let app = app.clone();
        async move {
            let resp = app.oneshot(r).await.unwrap();
            let robots = resp.headers().get("x-robots-tag").map(|v| v.to_str().unwrap().to_string());
            let status = resp.status();
            let body = resp.into_body().collect().await.unwrap().to_bytes();
            (status, body.len(), robots)
        }
    };

    // The login page exists but says nothing about what this is.
    let resp = app.clone().oneshot(get("/admin")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let html = String::from_utf8(resp.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    assert!(html.contains(r#"data-stealth="true""#) && html.contains(r#"data-name="""#));
    assert!(html.contains("<title></title>"), "the title is set by the page, neutral in discreet mode");
    for leak in ["Témoin", "SR_ADMIN_TOKEN", "{{"] {
        assert!(!html.contains(leak), "admin login page leaks {leak}");
    }

    // Without credentials everything else looks like an empty site.
    for path in ["/", "/api/v1/info", "/api/v1/admin/stats", "/api/v1/me", "/nope"] {
        let (s, len, robots) = raw(get(path)).await;
        assert_eq!((s, len), (StatusCode::NOT_FOUND, 0), "{path}");
        assert_eq!(robots.as_deref(), Some("noindex, nofollow"));
    }
    let anon_put = Request::put("/api/v1/objects/abcdefgh12/0").body(Body::from("x")).unwrap();
    assert_eq!(raw(anon_put).await.0, StatusCode::NOT_FOUND);
    let bad_admin = Request::get("/api/v1/admin/stats").header("authorization", "Bearer wrong").body(Body::empty()).unwrap();
    assert_eq!(raw(bad_admin).await.0, StatusCode::NOT_FOUND);

    // The admin token opens the API.
    let inv = invite(&app, serde_json::json!({ "label": "s" })).await;
    let code = inv["code"].as_str().unwrap().to_string();
    assert!(inv["link"].as_str().unwrap().contains("&i="));

    // An invitation holder may read /info (code typed with spaces is fine).
    let spaced: String = code.chars().collect::<Vec<_>>().chunks(5).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join(" ");
    let (s, info) = send(&app, Request::get("/api/v1/info").header("x-sr-invite", spaced).body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(info["server"], "temoin");
    let wrong = Request::get("/api/v1/info").header("x-sr-invite", "WRONGCODE").body(Body::empty()).unwrap();
    assert_eq!(raw(wrong).await.0, StatusCode::NOT_FOUND);

    // A registered device reads /info with a signed request, then uploads normally.
    let p = Phone::new();
    assert_eq!(send(&app, p.register_req(&code)).await.0, StatusCode::OK);
    assert_eq!(send(&app, p.signed("GET", "/api/v1/info", vec![], now())).await.0, StatusCode::OK);
    let obj = srec::seal::seal(&key, &envelope::encode(&header("stealth_sess_1", 0), b"hi"));
    assert_eq!(send(&app, p.put("stealth_sess_1", 0, obj)).await.0, StatusCode::CREATED);
    // An unknown key still gets a precise answer (the app needs it), only after signing properly.
    assert_eq!(send(&app, Phone::new().signed("GET", "/api/v1/me", vec![], now())).await.0, StatusCode::UNAUTHORIZED);
}
