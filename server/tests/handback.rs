//! Hand-back links: request code from the phone, upload by the operator, one-time download.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use http_body_util::BodyExt;
use srec::config::Config;
use srec::devices::{register_message, request_message};
use srec::envelope::{self, Header};
use srec::server::{router, AppState};
use srec::sharecrypt;
use tower::ServiceExt;

const ADMIN: &str = "Bearer admin-secret";

fn now() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

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

    fn register(&self, invite: &str) -> Request<Body> {
        let ts = now();
        let pk = STANDARD.encode(self.key.verifying_key().as_bytes());
        let sig = self.key.sign(register_message(invite, &pk, ts).as_bytes());
        let body = serde_json::json!({ "invite": invite, "public_key": pk, "timestamp": ts, "signature": STANDARD.encode(sig.to_bytes()) });
        Request::post("/api/v1/devices").header("content-type", "application/json").body(Body::from(body.to_string())).unwrap()
    }

    /// Signed request without body (the hash field is "-").
    fn signed(&self, method: &str, path: &str) -> Request<Body> {
        let ts = now();
        let sig = self.key.sign(request_message(method, path, None, ts, &self.id).as_bytes());
        Request::builder()
            .method(method)
            .uri(path)
            .header("x-sr-device", &self.id)
            .header("x-sr-timestamp", ts.to_string())
            .header("x-sr-signature", STANDARD.encode(sig.to_bytes()))
            .body(Body::empty())
            .unwrap()
    }

    fn put(&self, session: &str, seq: u32, body: Vec<u8>) -> Request<Body> {
        use sha2::{Digest, Sha256};
        let path = format!("/api/v1/objects/{session}/{seq}");
        let sha = hex::encode(Sha256::digest(&body));
        let ts = now();
        let sig = self.key.sign(request_message("PUT", &path, Some(&sha), ts, &self.id).as_bytes());
        Request::put(&path)
            .header("x-sr-device", &self.id)
            .header("x-sr-timestamp", ts.to_string())
            .header("x-sr-signature", STANDARD.encode(sig.to_bytes()))
            .header("x-sr-content-sha256", sha)
            .body(Body::from(body))
            .unwrap()
    }
}

async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
    let (s, b) = send_raw(app, req).await;
    (s, serde_json::from_slice(&b).unwrap_or(serde_json::Value::Null))
}

async fn send_raw(app: &axum::Router, req: Request<Body>) -> (StatusCode, Vec<u8>) {
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    (status, resp.into_body().collect().await.unwrap().to_bytes().to_vec())
}

fn admin(method: &str, path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", ADMIN)
        .header("content-type", "application/json")
        .body(if body.is_null() { Body::empty() } else { Body::from(body.to_string()) })
        .unwrap()
}

fn claimed(method: &str, path: &str, claim: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().method(method).uri(path);
    if let Some(c) = claim {
        b = b.header("x-sr-claim", c);
    }
    b.body(Body::empty()).unwrap()
}

fn put_part(id: &str, n: usize, offset: usize, bytes: &[u8]) -> Request<Body> {
    Request::put(format!("/api/v1/admin/shares/{id}/files/{n}?offset={offset}"))
        .header("authorization", ADMIN)
        .body(Body::from(bytes.to_vec()))
        .unwrap()
}

fn header(session: &str) -> Header {
    Header {
        v: 1,
        session: session.into(),
        seq: 0,
        kind: "video".into(),
        container: "mpegts".into(),
        is_final: true,
        created_at: "2026-10-19T12:00:00Z".into(),
        start_ms: None,
        duration_ms: None,
        device: None,
        location: None,
        video: None,
    }
}

#[tokio::test]
async fn hand_back_link() {
    let dir = tempfile::tempdir().unwrap();
    let id = age::x25519::Identity::generate();
    let recipient = id.to_public().to_string();
    let key = srec::seal::parse_recipient(&recipient).unwrap();
    let app = router(AppState::open(Config::for_tests(dir.path().to_owned(), recipient)).unwrap());

    let (_, inv) = send(&app, admin("POST", "/api/v1/admin/invites", serde_json::json!({ "label": "x" }))).await;
    let phone = Phone::new();
    assert_eq!(send(&app, phone.register(inv["code"].as_str().unwrap())).await.0, StatusCode::OK);
    let obj = srec::seal::seal(&key, &envelope::encode(&header("handback_sess1"), b"video"));
    assert_eq!(send(&app, phone.put("handback_sess1", 0, obj.clone())).await.0, StatusCode::CREATED);
    let original = dir.path().join("objects/ha/handback_sess1/000000.age");
    let moved = dir.path().join("returned/ha/handback_sess1/000000.age");
    assert!(original.exists());

    // The phone asks: a short code, stable until it is used.
    let (s, r) = send(&app, phone.signed("POST", "/api/v1/me/request")).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let code = r["code"].as_str().unwrap().to_string();
    assert_eq!(code.len(), 9, "XXXX-XXXX");
    assert!(r["link"].is_null());
    let (_, again) = send(&app, phone.signed("POST", "/api/v1/me/request")).await;
    assert_eq!(again["code"], code.as_str());
    assert_eq!(send(&app, claimed("POST", "/api/v1/me/request", None)).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(send(&app, Phone::new().signed("POST", "/api/v1/me/request")).await.0, StatusCode::UNAUTHORIZED);
    let (_, me) = send(&app, phone.signed("GET", "/api/v1/me")).await;
    assert_eq!(me["request"]["code"], code.as_str());

    // The operator looks the code up (any spelling) and finds the device. Admin only.
    let lookup = format!("/api/v1/admin/requests/{}", code.to_lowercase().replace('-', ""));
    let (s, req) = send(&app, admin("GET", &lookup, serde_json::Value::Null)).await;
    assert_eq!(s, StatusCode::OK, "{req}");
    assert_eq!((req["device_id"].as_str(), req["device_name"].as_str()), (Some(phone.id.as_str()), Some("Phone #1")));
    assert_eq!(req["objects"], 1);
    assert_eq!(send(&app, admin("GET", "/api/v1/admin/requests/ZZZZZZZZ", serde_json::Value::Null)).await.0, StatusCode::NOT_FOUND);
    assert_eq!(send(&app, claimed("GET", &lookup, None)).await.0, StatusCode::UNAUTHORIZED);

    // Browser hand-back: the admin page lists the device's encrypted objects and fetches
    // them as stored (ciphertext only; the key stays in the operator's browser).
    let (s, list) = send(&app, admin("GET", &format!("{lookup}/objects"), serde_json::Value::Null)).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    assert_eq!(list["device_id"], phone.id.as_str());
    assert_eq!(list["objects"].as_array().unwrap().len(), 1);
    assert_eq!(list["objects"][0]["session"], "handback_sess1");
    assert_eq!(list["upto_index"], 0);
    let (s, got) = send_raw(&app, admin("GET", "/api/v1/admin/objects/handback_sess1/0", serde_json::Value::Null)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(got, obj, "served exactly as stored");
    assert_eq!(send(&app, claimed("GET", "/api/v1/admin/objects/handback_sess1/0", None)).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(send(&app, admin("GET", "/api/v1/admin/objects/handback_sess1/9", serde_json::Value::Null)).await.0, StatusCode::NOT_FOUND);
    assert_eq!(send(&app, admin("GET", "/api/v1/admin/objects/..%2F..%2Fetc/0", serde_json::Value::Null)).await.0, StatusCode::BAD_REQUEST);

    // sr share: fresh key, upload in parts, seal.
    let k = sharecrypt::new_key();
    let manifest = br#"{"v":1,"files":[{"name":"a.mp4","type":"video/mp4"}]}"#.to_vec();
    let media: Vec<u8> = (0..(sharecrypt::CHUNK * 2 + 123)).map(|i| (i % 253) as u8).collect();
    let (c0, c1) = (sharecrypt::encrypt(&k, 0, &manifest), sharecrypt::encrypt(&k, 1, &media));
    let body = serde_json::json!({ "code": code, "files": [c0.len(), c1.len()], "days": 7 });
    let (s, created) = send(&app, admin("POST", "/api/v1/admin/shares", body)).await;
    assert_eq!(s, StatusCode::OK, "{created}");
    let sid = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["link"], format!("https://record.example.test/r/{sid}"));
    assert_eq!(send(&app, put_part(&sid, 0, 0, &c0)).await.0, StatusCode::OK);
    let half = c1.len() / 2;
    assert_eq!(send(&app, put_part(&sid, 1, 0, &c1[..half])).await.0, StatusCode::OK);
    // a hole is refused, a retried part is accepted
    assert_eq!(send(&app, put_part(&sid, 1, half + 10, &c1[half..])).await.0, StatusCode::CONFLICT);
    assert_eq!(send(&app, put_part(&sid, 1, 0, &c1[..half])).await.0, StatusCode::OK);
    let seal = format!("/api/v1/admin/shares/{sid}/seal");
    assert_eq!(send(&app, admin("POST", &seal, serde_json::Value::Null)).await.0, StatusCode::CONFLICT);
    let (s, e) = send(&app, claimed("POST", &format!("/r/{sid}/open"), None)).await;
    assert_eq!((s, e["error"].as_str()), (StatusCode::CONFLICT, Some("not_ready")));
    assert_eq!(send(&app, put_part(&sid, 1, half, &c1[half..])).await.0, StatusCode::OK);
    assert_eq!(send(&app, put_part(&sid, 1, c1.len(), b"extra")).await.0, StatusCode::PAYLOAD_TOO_LARGE);
    let (s, sealed) = send(&app, admin("POST", &seal, serde_json::Value::Null)).await;
    assert_eq!((s, sealed["state"].as_str()), (StatusCode::OK, Some("ready")));
    let (_, me) = send(&app, phone.signed("GET", "/api/v1/me")).await;
    assert_eq!(me["request"]["link"], "ready");

    // The page exists for this link only; unknown links are bare 404s.
    let (s, page) = send_raw(&app, claimed("GET", &format!("/r/{sid}"), None)).await;
    assert_eq!(s, StatusCode::OK);
    assert!(String::from_utf8_lossy(&page).contains("AES-GCM"));
    assert_eq!(send_raw(&app, claimed("GET", "/r/AAAAAAAAAAAAAAAAAAAAAA", None)).await.0, StatusCode::NOT_FOUND);
    let (_, meta) = send(&app, claimed("GET", &format!("/r/{sid}/meta"), None)).await;
    assert_eq!((meta["state"].as_str(), meta["files"].as_u64()), (Some("ready"), Some(1)));

    // Manifest readable before opening (key check), media not.
    let (s, m0) = send_raw(&app, claimed("GET", &format!("/r/{sid}/f/0"), None)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(sharecrypt::decrypt(&k, 0, &m0).unwrap(), manifest);
    assert_eq!(send(&app, claimed("GET", &format!("/r/{sid}/f/1"), None)).await.0, StatusCode::GONE);

    // First opening wins; a second browser is refused; the first one can resume.
    let open = format!("/r/{sid}/open");
    let (s, opened) = send(&app, claimed("POST", &open, None)).await;
    assert_eq!(s, StatusCode::OK, "{opened}");
    let claim = opened["claim"].as_str().unwrap().to_string();
    let (s, e) = send(&app, claimed("POST", &open, None)).await;
    assert_eq!((s, e["error"].as_str()), (StatusCode::GONE, Some("used")));
    assert_eq!(send(&app, claimed("POST", &open, Some("forged"))).await.0, StatusCode::GONE);
    assert_eq!(send(&app, claimed("POST", &open, Some(&claim))).await.0, StatusCode::OK);
    assert_eq!(send(&app, claimed("GET", &format!("/r/{sid}/f/1"), Some("forged"))).await.0, StatusCode::GONE);
    let (s, body) = send_raw(&app, claimed("GET", &format!("/r/{sid}/f/1"), Some(&claim))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(sharecrypt::decrypt(&k, 1, &body).unwrap(), media);
    assert_eq!(send(&app, claimed("GET", &format!("/r/{sid}/f/2"), Some(&claim))).await.0, StatusCode::NOT_FOUND);

    // Done: the link stops working; NOTHING is deleted. The link copy is archived, the
    // recordings move to returned/ and stop counting against the quota.
    let (_, me) = send(&app, phone.signed("GET", "/api/v1/me")).await;
    assert_eq!((me["objects"].as_u64(), me["used_bytes"].as_u64()), (Some(1), Some(std::fs::metadata(&original).unwrap().len())));
    assert_eq!(send(&app, claimed("POST", &format!("/r/{sid}/done"), Some(&claim))).await.0, StatusCode::OK);
    assert!(!dir.path().join("shares").join(&sid).exists());
    assert!(dir.path().join("shares-archive").join(&sid).join("1.bin").exists());
    assert!(!original.exists() && moved.exists(), "recording moved, not deleted");
    let (_, me) = send(&app, phone.signed("GET", "/api/v1/me")).await;
    assert_eq!((me["objects"].as_u64(), me["used_bytes"].as_u64()), (Some(0), Some(0)), "quota freed");
    let log = std::fs::read_to_string(dir.path().join("returned.jsonl")).unwrap();
    assert_eq!(log.lines().count(), 1);
    assert_eq!(srec::ledger::verify_file(&dir.path().join("ledger.jsonl")).unwrap().len(), 1, "ledger untouched");
    // A late retry of the same object is a duplicate, not a second copy.
    let (s, r) = send(&app, phone.put("handback_sess1", 0, obj)).await;
    assert_eq!((s, r["duplicate"].as_bool()), (StatusCode::OK, Some(true)));
    assert!(!original.exists());
    // New recordings count normally.
    let obj2 = srec::seal::seal(&key, &envelope::encode(&header("handback_sess2"), b"more"));
    assert_eq!(send(&app, phone.put("handback_sess2", 0, obj2)).await.0, StatusCode::CREATED);
    let (_, me) = send(&app, phone.signed("GET", "/api/v1/me")).await;
    assert_eq!(me["objects"], 1);
    let (_, meta) = send(&app, claimed("GET", &format!("/r/{sid}/meta"), None)).await;
    assert_eq!(meta["state"], "done");
    assert_eq!(send_raw(&app, claimed("GET", &format!("/r/{sid}"), None)).await.0, StatusCode::OK);
    let (s, e) = send(&app, claimed("POST", &open, Some(&claim))).await;
    assert_eq!((s, e["error"].as_str()), (StatusCode::GONE, Some("done")));
    let (_, me) = send(&app, phone.signed("GET", "/api/v1/me")).await;
    assert_eq!(me["request"]["link"], "done");

    // Asking again after that gives a new code.
    let (_, r2) = send(&app, phone.signed("POST", "/api/v1/me/request")).await;
    assert_ne!(r2["code"], code.as_str());
    let code2 = r2["code"].as_str().unwrap().to_string();

    // A second share for one request replaces the first; revocation deletes the content.
    let mk = |code: &str| admin("POST", "/api/v1/admin/shares", serde_json::json!({ "code": code, "files": [c0.len()] }));
    let (_, a) = send(&app, mk(&code2)).await;
    let a = a["id"].as_str().unwrap().to_string();
    assert_eq!(send(&app, put_part(&a, 0, 0, &c0)).await.0, StatusCode::OK);
    assert_eq!(send(&app, admin("POST", &format!("/api/v1/admin/shares/{a}/seal"), serde_json::Value::Null)).await.0, StatusCode::OK);
    let (_, b) = send(&app, mk(&code2)).await;
    let b = b["id"].as_str().unwrap().to_string();
    let (_, meta) = send(&app, claimed("GET", &format!("/r/{a}/meta"), None)).await;
    assert_eq!(meta["state"], "revoked");
    assert!(!dir.path().join("shares").join(&a).exists());
    assert!(dir.path().join("shares-archive").join(&a).exists());
    // a revoked link moves nothing
    assert!(dir.path().join("objects/ha/handback_sess2/000000.age").exists());
    assert_eq!(send(&app, admin("POST", &format!("/api/v1/admin/shares/{b}/revoke"), serde_json::Value::Null)).await.0, StatusCode::OK);
    assert!(!dir.path().join("shares").join(&b).exists());
    let (_, list) = send(&app, admin("GET", "/api/v1/admin/requests", serde_json::Value::Null)).await;
    assert_eq!(list["requests"].as_array().unwrap().len(), 2);
    assert_eq!(list["requests"][0]["share"]["state"], "revoked");

    // Cancelled requests cannot get a link.
    assert_eq!(send(&app, admin("POST", &format!("/api/v1/admin/requests/{code2}/cancel"), serde_json::Value::Null)).await.0, StatusCode::OK);
    assert_eq!(send(&app, mk(&code2)).await.0, StatusCode::BAD_REQUEST);
}

#[test]
fn shares_expire_and_survive_restart() {
    use srec::shares::{NewShare, ShareError, ShareStore, State};
    let dir = tempfile::tempdir().unwrap();
    let store = ShareStore::open(dir.path()).unwrap();
    let r = store.request_for_device("dev1").unwrap();
    let sh = store.create(NewShare { request_code: Some(r.code.clone()), upto_index: None, label: String::new(), files: vec![3], days: 1 }).unwrap();
    store.append(&sh.id, 0, 0, b"abc").unwrap();
    store.seal(&sh.id).unwrap();
    drop(store);

    // Restart keeps it; once its expiry is past, the sweeper deletes the content.
    let path = dir.path().join("shares.json");
    let mut j: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(j["shares"][0]["state"], "ready");
    j["shares"][0]["expires_at"] = "2020-01-01T00:00:00Z".into();
    std::fs::write(&path, serde_json::to_vec(&j).unwrap()).unwrap();
    std::fs::create_dir_all(dir.path().join("shares/stray")).unwrap();
    let store = ShareStore::open(dir.path()).unwrap();
    assert_eq!(store.get(&sh.id).unwrap().state, State::Expired);
    assert!(!dir.path().join("shares").join(&sh.id).exists());
    assert!(!dir.path().join("shares/stray").exists());
    assert!(dir.path().join("shares-archive/stray").exists(), "archived, not deleted");
    assert!(dir.path().join("shares-archive").join(&sh.id).join("0.bin").exists());
    assert_eq!(store.claim(&sh.id, None).unwrap_err(), ShareError::Expired);
    // after an expired link, the device may ask again
    assert_ne!(store.request_for_device("dev1").unwrap().code, r.code);
}

#[tokio::test]
async fn opened_window_closes() {
    use srec::shares::{NewShare, ShareStore, State};
    let dir = tempfile::tempdir().unwrap();
    let store = ShareStore::open(dir.path()).unwrap();
    let sh = store.create(NewShare { request_code: None, upto_index: None, label: String::new(), files: vec![1], days: 7 }).unwrap();
    store.append(&sh.id, 0, 0, b"x").unwrap();
    store.seal(&sh.id).unwrap();
    let (tok, _) = store.claim(&sh.id, None).unwrap();
    assert!(store.file(&sh.id, 0, Some(&tok)).is_ok());
    drop(store);
    let path = dir.path().join("shares.json");
    let mut j: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    j["shares"][0]["opened_at"] = "2020-01-01T00:00:00Z".into();
    std::fs::write(&path, serde_json::to_vec(&j).unwrap()).unwrap();
    let store = ShareStore::open(dir.path()).unwrap();
    assert_eq!(store.get(&sh.id).unwrap().state, State::Done);
    assert!(store.file(&sh.id, 0, Some(&tok)).is_err());
    assert!(!dir.path().join("shares").join(&sh.id).exists());
}

#[tokio::test]
async fn stealth_hides_share_routes() {
    let dir = tempfile::tempdir().unwrap();
    let id = age::x25519::Identity::generate();
    let mut cfg = Config::for_tests(dir.path().to_owned(), id.to_public().to_string());
    cfg.stealth = true;
    let app = router(AppState::open(cfg).unwrap());
    let fake = "/r/AAAAAAAAAAAAAAAAAAAAAA";
    for (m, p) in [
        ("GET", fake.to_string()),
        ("GET", format!("{fake}/meta")),
        ("POST", format!("{fake}/open")),
        ("GET", format!("{fake}/f/0")),
        ("POST", "/api/v1/me/request".to_string()),
        ("GET", "/api/v1/admin/requests".to_string()),
    ] {
        let (s, body) = send_raw(&app, claimed(m, &p, None)).await;
        assert_eq!((s, body.len()), (StatusCode::NOT_FOUND, 0), "{m} {p}");
    }
}
