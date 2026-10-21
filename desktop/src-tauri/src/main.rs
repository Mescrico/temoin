//! Témoin server for a home PC.
//!
//! One window: first-run wizard (server name, data folder, passphrase-protected key),
//! then start/stop, public address (Cloudflare quick tunnel by default, or built-in HTTPS
//! with the user's own domain), invitation QR codes, recovery of the recordings and
//! hand-back links. The server itself is the `srec` library, running in-process.
//!
//! The private key lives next to the server here (a single PC), so it is always stored
//! encrypted with a passphrase and only decrypted for the time of a recovery.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use age::secrecy::SecretString;
use base64::Engine;
use serde::{Deserialize, Serialize};
use srec::ops::share::Admin;
use srec::server::AppState;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder, WindowEvent};

const DEFAULT_PORT: u16 = 47180;

/// What the app remembers between runs (`config.json` in the app config folder).
/// Never the private key or the passphrase.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DesktopConfig {
    name: String,
    data_dir: PathBuf,
    port: u16,
    admin_token: String,
    recipient: String,
    /// "tunnel" (Cloudflare quick tunnel) or "https" (built-in Let's Encrypt).
    reach: String,
    #[serde(default)]
    acme_domain: Option<String>,
    #[serde(default)]
    acme_email: Option<String>,
    #[serde(default)]
    log_ips: bool,
    /// Start the server as soon as the app opens.
    #[serde(default = "yes")]
    autostart: bool,
    /// Where "Recover" writes the decrypted files.
    #[serde(default)]
    recovery_dir: Option<PathBuf>,
}

fn yes() -> bool {
    true
}

struct Running {
    state: Arc<AppState>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    _awake: Option<keepawake::KeepAwake>,
}

#[derive(Default)]
struct Ctl {
    running: Mutex<Option<Running>>,
    last_error: Mutex<Option<String>>,
}

type CmdResult<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    format!("{e:#}")
}

/// Settings and key folder. `SR_DESKTOP_HOME` overrides it (tests, portable install).
fn config_dir(app: &AppHandle) -> PathBuf {
    match std::env::var_os("SR_DESKTOP_HOME") {
        Some(home) => PathBuf::from(home).join("config"),
        None => app.path().app_config_dir().expect("app config dir"),
    }
}

fn config_path(app: &AppHandle) -> PathBuf {
    config_dir(app).join("config.json")
}

fn identity_path(app: &AppHandle) -> PathBuf {
    config_dir(app).join("identity.age")
}

fn load_config(app: &AppHandle) -> Option<DesktopConfig> {
    serde_json::from_slice(&std::fs::read(config_path(app)).ok()?).ok()
}

fn save_config(app: &AppHandle, c: &DesktopConfig) -> anyhow::Result<()> {
    std::fs::create_dir_all(config_dir(app))?;
    let p = config_path(app);
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(c)?)?;
    std::fs::rename(tmp, p)?;
    Ok(())
}

fn local_base(c: &DesktopConfig) -> String {
    format!("http://127.0.0.1:{}", c.port)
}

fn qr_svg(data: &str) -> String {
    qrcode::QrCode::with_error_correction_level(data.as_bytes(), qrcode::EcLevel::M)
        .map(|q| q.render::<qrcode::render::svg::Color>().min_dimensions(300, 300).quiet_zone(true).build())
        .unwrap_or_default()
}

fn cloudflared_path(app: &AppHandle) -> PathBuf {
    let name = if cfg!(windows) { "cloudflared.exe" } else { "cloudflared" };
    app.path()
        .resource_dir()
        .map(|d| d.join(name))
        .ok()
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from(name))
}

fn passphrase(p: String) -> CmdResult<SecretString> {
    if p.is_empty() {
        return Err("@no_passphrase".into());
    }
    Ok(SecretString::from(p))
}

// ------------------------------------------------------------------ setup

#[derive(Serialize)]
struct Overview {
    configured: bool,
    default_data_dir: PathBuf,
    config: Option<PublicConfig>,
}

#[derive(Serialize)]
struct PublicConfig {
    name: String,
    data_dir: PathBuf,
    port: u16,
    reach: String,
    acme_domain: Option<String>,
    acme_email: Option<String>,
    fingerprint: String,
    identity_file: PathBuf,
    autostart: bool,
    recovery_dir: PathBuf,
}

#[tauri::command]
fn overview(app: AppHandle) -> Overview {
    let config = load_config(&app);
    Overview {
        configured: config.is_some() && identity_path(&app).exists(),
        default_data_dir: match std::env::var_os("SR_DESKTOP_HOME") {
            Some(home) => PathBuf::from(home).join("Témoin"),
            None => app.path().document_dir().unwrap_or_else(|_| PathBuf::from(".")).join("Témoin"),
        },
        config: config.map(|c| PublicConfig {
            fingerprint: srec::seal::fingerprint(&c.recipient),
            identity_file: identity_path(&app),
            autostart: c.autostart,
            recovery_dir: c.recovery_dir.clone().unwrap_or_else(|| c.data_dir.join("Recovered")),
            name: c.name,
            data_dir: c.data_dir,
            port: c.port,
            reach: c.reach,
            acme_domain: c.acme_domain,
            acme_email: c.acme_email,
        }),
    }
}

/// First run: creates the key (encrypted with the passphrase) and the configuration.
#[tauri::command]
fn setup(app: AppHandle, name: String, data_dir: PathBuf, passphrase: String) -> CmdResult<String> {
    if passphrase.chars().count() < 10 {
        return Err("@passphrase_short".into());
    }
    let id = identity_path(&app);
    if id.exists() {
        return Err("@key_exists".into());
    }
    std::fs::create_dir_all(&data_dir).map_err(err)?;
    std::fs::create_dir_all(config_dir(&app)).map_err(err)?;
    let recipient = srec::ops::write_identity(&id, Some(&SecretString::from(passphrase))).map_err(err)?;
    let c = DesktopConfig {
        name: if name.trim().is_empty() { "Témoin".into() } else { name.trim().chars().take(60).collect() },
        data_dir,
        port: DEFAULT_PORT,
        admin_token: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>()),
        recipient: recipient.clone(),
        reach: "tunnel".into(),
        acme_domain: None,
        acme_email: None,
        log_ips: false,
        autostart: true,
        recovery_dir: None,
    };
    save_config(&app, &c).map_err(err)?;
    sync_login_item(&app, c.autostart);
    Ok(srec::seal::fingerprint(&recipient))
}

#[tauri::command]
fn export_key(app: AppHandle, dest: PathBuf) -> CmdResult<()> {
    std::fs::copy(identity_path(&app), dest).map_err(err)?;
    Ok(())
}

#[tauri::command]
fn save_settings(
    app: AppHandle,
    ctl: State<Ctl>,
    name: String,
    reach: String,
    acme_domain: Option<String>,
    acme_email: Option<String>,
    autostart: Option<bool>,
    recovery_dir: Option<PathBuf>,
) -> CmdResult<()> {
    let mut c = load_config(&app).ok_or("@not_configured")?;
    // Settings that do not touch the running server can change at any time.
    if let Some(a) = autostart {
        c.autostart = a;
        sync_login_item(&app, a);
    }
    if let Some(d) = recovery_dir.filter(|d| !d.as_os_str().is_empty()) {
        c.recovery_dir = Some(d);
    }
    let reach_changed = (if reach == "https" { "https" } else { "tunnel" }) != c.reach
        || acme_domain.as_deref().map(str::trim).filter(|v| !v.is_empty()) != c.acme_domain.as_deref()
        || name.trim() != c.name;
    if reach_changed && ctl.running.lock().unwrap().is_some() {
        return Err("@stop_first".into());
    }
    if !name.trim().is_empty() {
        c.name = name.trim().chars().take(60).collect();
    }
    c.reach = if reach == "https" { "https".into() } else { "tunnel".into() };
    let clean = |s: Option<String>| s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    c.acme_domain = clean(acme_domain);
    c.acme_email = clean(acme_email);
    if c.reach == "https" && c.acme_domain.is_none() {
        return Err("@domain_required".into());
    }
    save_config(&app, &c).map_err(err)
}

// ------------------------------------------------------------------ server

#[tauri::command]
fn start(app: AppHandle, ctl: State<Ctl>) -> CmdResult<()> {
    let mut running = ctl.running.lock().unwrap();
    if running.is_some() {
        return Ok(());
    }
    let c = load_config(&app).ok_or("@not_configured")?;
    let mut cfg = srec::config::Config::with_defaults(c.data_dir.clone(), c.recipient.clone());
    cfg.name = c.name.clone();
    cfg.bind = format!("127.0.0.1:{}", c.port);
    cfg.admin_token = Some(c.admin_token.clone());
    cfg.log_ips = c.log_ips;
    match c.reach.as_str() {
        "https" => {
            cfg.acme_domain = c.acme_domain.clone();
            cfg.acme_email = c.acme_email.clone();
        }
        _ => {
            cfg.tunnel = "cloudflare".into();
            cfg.cloudflared = cloudflared_path(&app);
            // Only cloudflared (on this PC) can reach the 127.0.0.1 listener.
            cfg.trust_proxy = true;
        }
    }
    std::fs::create_dir_all(&c.data_dir).map_err(err)?;
    let state = AppState::open(cfg).map_err(err)?;
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let st = state.clone();
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        let res = srec::runtime::serve(st, async {
            let _ = rx.await;
        })
        .await;
        if let Err(e) = res {
            tracing::error!("server stopped: {e:#}");
            let ctl = app2.state::<Ctl>();
            *ctl.last_error.lock().unwrap() = Some(format!("{e:#}"));
            ctl.running.lock().unwrap().take();
        }
    });
    // A PC that falls asleep drops every upload: keep it awake while serving.
    let awake = keepawake::Builder::default()
        .idle(true)
        .reason("Témoin is receiving recordings")
        .app_name("Témoin Server")
        .app_reverse_domain("io.github.temoin-app.server")
        .create()
        .ok();
    *ctl.last_error.lock().unwrap() = None;
    *running = Some(Running { state, stop: Some(tx), _awake: awake });
    Ok(())
}

#[tauri::command]
fn stop(ctl: State<Ctl>) -> CmdResult<()> {
    if let Some(mut r) = ctl.running.lock().unwrap().take() {
        if let Some(tx) = r.stop.take() {
            let _ = tx.send(());
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct Status {
    running: bool,
    error: Option<String>,
    public_url: Option<String>,
    tunnel: Option<srec::tunnel::TunnelStatus>,
    accepting: bool,
    devices: usize,
    objects: u64,
    bytes: u64,
    last_received_at: Option<String>,
    free_bytes: Option<u64>,
    captures: u64,
    /// Objects received since the last "Recover".
    new_objects: u64,
    /// Hand-back requests waiting for a link.
    pending_requests: usize,
}

#[tauri::command]
async fn status(app: AppHandle, ctl: State<'_, Ctl>) -> CmdResult<Status> {
    let state = ctl.running.lock().unwrap().as_ref().map(|r| r.state.clone());
    let error = ctl.last_error.lock().unwrap().clone();
    let Some(st) = state else {
        return Ok(Status {
            running: false,
            error,
            public_url: None,
            tunnel: None,
            accepting: false,
            devices: 0,
            objects: 0,
            bytes: 0,
            last_received_at: None,
            free_bytes: None,
            captures: 0,
            new_objects: 0,
            pending_requests: 0,
        });
    };
    let recovered_upto: u64 = std::fs::read_to_string(config_dir(&app).join("recovery-index.txt"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let pending_requests = st.shares.requests().iter().filter(|r| !r.cancelled && r.share_id.is_none()).count();
    let tunnel = st.tunnel();
    let public = st.public_url();
    let l = st.ledger.lock().await;
    Ok(Status {
        running: true,
        error,
        // Not reachable from phones until the tunnel/certificate is up.
        public_url: (tunnel.mode == "off" || tunnel.state == "up").then_some(public),
        tunnel: Some(tunnel),
        accepting: st.accepting.load(std::sync::atomic::Ordering::Relaxed),
        devices: st.registry.devices().len(),
        objects: l.next_index,
        bytes: l.total_bytes,
        last_received_at: l.last_received_at.clone(),
        free_bytes: srec_free(&st),
        captures: l.sessions,
        new_objects: l.next_index.saturating_sub(recovered_upto),
        pending_requests,
    })
}

fn srec_free(st: &AppState) -> Option<u64> {
    st.cfg.data_dir.exists().then(|| fs_free(&st.cfg.data_dir)).flatten()
}

fn fs_free(p: &Path) -> Option<u64> {
    // Same call as the server's disk reserve check.
    srec::ops::available_space(p)
}

fn admin_client(app: &AppHandle) -> CmdResult<(Admin, DesktopConfig)> {
    let c = load_config(app).ok_or("@not_configured")?;
    Ok((Admin::with_token(&local_base(&c), c.admin_token.clone()), c))
}

#[derive(Serialize, Clone)]
struct Invite {
    code: String,
    link: String,
    svg: String,
    expires_at: Option<String>,
    max_uses: Option<u32>,
}

/// New invitation (QR to scan in the app). Defaults suited to one event.
#[tauri::command]
async fn invite(app: AppHandle, label: String, hours: u32, max_uses: u32) -> CmdResult<Invite> {
    let (admin, _) = admin_client(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let r = admin
            .post_json(
                "/api/v1/admin/invites",
                serde_json::json!({
                    "label": if label.trim().is_empty() { "Invitation".to_string() } else { label },
                    "expires_hours": (hours > 0).then_some(hours),
                    "max_uses": (max_uses > 0).then_some(max_uses),
                }),
            )
            .map_err(err)?;
        Ok(Invite {
            code: r["code"].as_str().unwrap_or_default().to_string(),
            link: r["link"].as_str().unwrap_or_default().to_string(),
            svg: r["svg"].as_str().unwrap_or_default().to_string(),
            expires_at: r["expires_at"].as_str().map(String::from),
            max_uses: r["max_uses"].as_u64().map(|n| n as u32),
        })
    })
    .await
    .map_err(err)?
}

/// Free space where `path` would be stored (setup wizard).
#[tauri::command]
fn free_space(path: PathBuf) -> Option<u64> {
    let mut p = path.as_path();
    loop {
        if p.exists() {
            return fs_free(p);
        }
        p = p.parent()?;
    }
}

/// GET on the local admin API, passed through to the page.
async fn admin_get(app: &AppHandle, path: &'static str) -> CmdResult<serde_json::Value> {
    let (admin, _) = admin_client(app)?;
    tauri::async_runtime::spawn_blocking(move || admin.get_json(path).map_err(err)).await.map_err(err)?
}

async fn admin_post(app: &AppHandle, path: String, body: serde_json::Value) -> CmdResult<serde_json::Value> {
    let (admin, _) = admin_client(app)?;
    tauri::async_runtime::spawn_blocking(move || admin.post_json(&path, body).map_err(err)).await.map_err(err)?
}

/// Registered phones (pseudonym, usage, ban state).
#[tauri::command]
async fn devices(app: AppHandle) -> CmdResult<serde_json::Value> {
    admin_get(&app, "/api/v1/admin/devices").await
}

#[tauri::command]
async fn update_device(app: AppHandle, id: String, banned: Option<bool>, name: Option<String>) -> CmdResult<serde_json::Value> {
    if !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("bad device id".into());
    }
    admin_post(&app, format!("/api/v1/admin/devices/{id}"), serde_json::json!({ "banned": banned, "name": name })).await
}

/// Hand-back requests and links.
#[tauri::command]
async fn requests(app: AppHandle) -> CmdResult<serde_json::Value> {
    admin_get(&app, "/api/v1/admin/requests").await
}

#[tauri::command]
async fn revoke_share(app: AppHandle, id: String) -> CmdResult<serde_json::Value> {
    if !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err("bad link id".into());
    }
    admin_post(&app, format!("/api/v1/admin/shares/{id}/revoke"), serde_json::Value::Null).await
}

/// The newest invitation still valid, with its QR code (it follows the current address).
#[tauri::command]
async fn current_invite(app: AppHandle) -> CmdResult<Option<Invite>> {
    let (admin, _) = admin_client(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let list = admin.get_json("/api/v1/admin/invites").map_err(err)?;
        let Some(i) = list["invites"].as_array().and_then(|a| a.iter().find(|i| i["active"].as_bool() == Some(true))).cloned() else {
            return Ok(None);
        };
        let link = i["link"].as_str().unwrap_or_default().to_string();
        Ok(Some(Invite {
            code: i["code"].as_str().unwrap_or_default().to_string(),
            svg: qr_svg(&link),
            link,
            expires_at: i["expires_at"].as_str().map(String::from),
            max_uses: i["max_uses"].as_u64().map(|n| n as u32),
        }))
    })
    .await
    .map_err(err)?
}

/// The full admin page (invitations, devices, journal, hand-back requests), logged in.
/// Async on purpose: creating a window from a synchronous command deadlocks on Windows.
#[tauri::command]
async fn open_admin(app: AppHandle) -> CmdResult<()> {
    let c = load_config(&app).ok_or("@not_configured")?;
    if let Some(w) = app.get_webview_window("admin") {
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    let url = format!("{}/admin", local_base(&c)).parse().map_err(err)?;
    let token = serde_json::to_string(&c.admin_token).map_err(err)?;
    WebviewWindowBuilder::new(&app, "admin", WebviewUrl::External(url))
        .title("Administration — Témoin")
        .inner_size(1100.0, 820.0)
        .initialization_script(&format!("try {{ sessionStorage.setItem('sr-admin', {token}); }} catch (e) {{}}"))
        .build()
        .map_err(err)?;
    Ok(())
}

// ------------------------------------------------------------------ operator

#[derive(Serialize)]
struct Recovery {
    folder: Option<PathBuf>,
    lines: Vec<String>,
    sessions: usize,
    unreadable: usize,
    nothing_new: bool,
}

/// Decrypts what arrived since the last recovery into `<out>/<date>`.
#[tauri::command]
async fn recover(app: AppHandle, passphrase: String, out: PathBuf, all: bool) -> CmdResult<Recovery> {
    let pass = self::passphrase(passphrase)?;
    let c = load_config(&app).ok_or("@not_configured")?;
    let id_path = identity_path(&app);
    let state_file = config_dir(&app).join("recovery-index.txt");
    tauri::async_runtime::spawn_blocking(move || {
        let identity = srec::ops::load_identity(&id_path, Some(&pass)).map_err(|e| {
            if format!("{e}").contains("wrong passphrase") { "@wrong_passphrase".to_string() } else { err(e) }
        })?;
        if all {
            let _ = std::fs::remove_file(&state_file);
        }
        let ledger = c.data_dir.join("ledger.jsonl");
        if !ledger.exists() {
            return Ok(Recovery { folder: None, lines: vec![], sessions: 0, unreadable: 0, nothing_new: true });
        }
        let window = srec::ops::Window::resolve(&c.data_dir, None, Some(&state_file)).map_err(err)?.ok_or("index")?;
        if window.objects.is_empty() {
            return Ok(Recovery { folder: None, lines: vec![], sessions: 0, unreadable: 0, nothing_new: true });
        }
        let stamp = time::OffsetDateTime::now_local()
            .unwrap_or_else(|_| time::OffsetDateTime::now_utc())
            .format(time::macros::format_description!("[year]-[month]-[day]_[hour][minute]"))
            .map_err(err)?;
        let folder = out.join(stamp);
        let report = srec::ops::decrypt(&identity, &c.data_dir, &folder, true, false, Some(&window.objects)).map_err(err)?;
        window.save(&state_file).map_err(err)?;
        Ok(Recovery { folder: Some(folder), lines: report.lines, sessions: report.sessions, unreadable: report.unreadable, nothing_new: false })
    })
    .await
    .map_err(err)?
}

#[derive(Serialize)]
struct Lookup {
    code: String,
    device_name: Option<String>,
    objects: u64,
    bytes: u64,
    cancelled: bool,
}

#[tauri::command]
async fn lookup_request(app: AppHandle, code: String) -> CmdResult<Lookup> {
    let (admin, _) = admin_client(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let r = admin.request(&code).map_err(|e| {
            if format!("{e}").contains("404") { "@unknown_code".to_string() } else { err(e) }
        })?;
        Ok(Lookup {
            code: r["code"].as_str().unwrap_or_default().to_string(),
            device_name: r["device_name"].as_str().map(String::from),
            objects: r["objects"].as_u64().unwrap_or(0),
            bytes: r["bytes"].as_u64().unwrap_or(0),
            cancelled: r["cancelled"].as_bool().unwrap_or(false),
        })
    })
    .await
    .map_err(err)?
}

#[derive(Serialize)]
struct Link {
    link: String,
    svg: String,
}

/// One-time link with that person's recordings (decrypted here, re-encrypted with a fresh key).
#[tauri::command]
async fn hand_back(app: AppHandle, code: String, passphrase: String, days: u32) -> CmdResult<Link> {
    let pass = self::passphrase(passphrase)?;
    let (admin, c) = admin_client(&app)?;
    let id_path = identity_path(&app);
    tauri::async_runtime::spawn_blocking(move || {
        let identity = srec::ops::load_identity(&id_path, Some(&pass)).map_err(|e| {
            if format!("{e}").contains("wrong passphrase") { "@wrong_passphrase".to_string() } else { err(e) }
        })?;
        let link = srec::ops::share::share(&identity, &c.data_dir, &admin, &code, days.clamp(1, 30), None).map_err(err)?;
        Ok(Link { svg: qr_svg(&link), link })
    })
    .await
    .map_err(err)?
}

// ------------------------------------------------------------------ app

/// Argument given by the Windows login entry: start hidden in the notification area.
const BACKGROUND_ARG: &str = "--background";

/// "Start automatically" also opens the app (hidden) when the user logs in, so the
/// server comes back after a reboot. Not touched by test runs (SR_DESKTOP_HOME).
fn sync_login_item(app: &AppHandle, on: bool) {
    use tauri_plugin_autostart::ManagerExt;
    if std::env::var_os("SR_DESKTOP_HOME").is_some() {
        return;
    }
    let launcher = app.autolaunch();
    let result = if on { launcher.enable() } else { launcher.disable() };
    if let Err(e) = result {
        tracing::warn!("login item: {e}");
    }
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("SR_LOG").unwrap_or_else(|_| "info".into()))
        .init();
    tauri::Builder::default()
        // A second launch (Start menu while running in the background) shows the open window.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![BACKGROUND_ARG]),
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Ctl::default())
        .setup(|app| {
            // Keep the login entry in step with the setting (and with the exe path after an update).
            if let Some(c) = load_config(app.handle()) {
                sync_login_item(app.handle(), c.autostart);
            }
            if !std::env::args().any(|a| a == BACKGROUND_ARG) {
                show_main(app.handle());
            }
            // Same rule as the window: French on a French system, English otherwise.
            let fr = sys_locale::get_locale().is_some_and(|l| l.to_ascii_lowercase().starts_with("fr"));
            let (open_label, quit_label, tip) = if fr {
                ("Ouvrir", "Arrêter le serveur et quitter", "Témoin — serveur")
            } else {
                ("Open", "Stop the server and quit", "Témoin — server")
            };
            let open = MenuItem::with_id(app, "open", open_label, true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", quit_label, true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;
            TrayIconBuilder::with_id("tray")
                .icon(app.default_window_icon().cloned().expect("icon"))
                .tooltip(tip)
                .menu(&menu)
                // macOS: the menu bar icon opens its menu on click (no double-click there).
                .show_menu_on_left_click(cfg!(target_os = "macos"))
                .on_menu_event(|app, e| match e.id.as_ref() {
                    "open" => show_main(app),
                    "quit" => {
                        let _ = stop(app.state::<Ctl>());
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, e| {
                    if let tauri::tray::TrayIconEvent::DoubleClick { .. } = e {
                        show_main(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window keeps the server running in the notification area.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" && window.state::<Ctl>().running.lock().unwrap().is_some() {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            overview, setup, export_key, save_settings, start, stop, status, invite, open_admin,
            recover, lookup_request, hand_back, free_space, devices, update_device, requests,
            revoke_share, current_invite
        ])
        .build(tauri::generate_context!())
        .expect("error while building the app")
        .run(|_app, _event| {
            // macOS: clicking the Dock icon brings the hidden window back.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = _event {
                show_main(_app);
            }
        });
}
