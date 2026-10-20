use std::path::PathBuf;

use anyhow::Context;

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: String,
    pub data_dir: PathBuf,
    pub name: String,
    pub public_url: Option<String>,
    pub recipient: String,
    pub admin_token: Option<String>,
    /// Path of the admin page (keep it secret in stealth mode).
    pub admin_path: String,
    /// Stealth: nothing on the server reveals what it is to someone without an
    /// invitation, a registered device key or the admin token (plain 404s).
    pub stealth: bool,
    /// Whole-request deadline (anti slowloris). Default 300 s.
    pub request_timeout_secs: u64,
    pub accepting_default: bool,
    pub max_object_bytes: u64,
    pub device_quota_bytes: u64,
    pub max_total_bytes: Option<u64>,
    pub min_free_bytes: u64,
    pub max_video_height: u32,
    pub default_video_height: u32,
    pub video_bitrate_720: u32,
    pub video_bitrate_1080: u32,
    pub photo_max_pixels: u32,
    /// Take the client IP from X-Real-IP / X-Forwarded-For (only behind a trusted proxy).
    pub trust_proxy: bool,
    /// Record client IPs in the journal.
    pub log_ips: bool,
    /// "off" or "cloudflare" (quick tunnel, see `tunnel.rs`).
    pub tunnel: String,
    /// cloudflared executable (default: `cloudflared` on the PATH).
    pub cloudflared: PathBuf,
    /// Built-in HTTPS with a Let's Encrypt certificate for this domain (port forwarding needed).
    pub acme_domain: Option<String>,
    pub acme_email: Option<String>,
    /// HTTPS listen address when `acme_domain` is set.
    pub https_bind: String,
    /// Use the Let's Encrypt staging CA (tests).
    pub acme_staging: bool,
}

const GIB: u64 = 1024 * 1024 * 1024;

impl Config {
    /// Reads `SR_*` environment variables.
    pub fn from_env() -> anyhow::Result<Self> {
        let recipient = match (var("SR_RECIPIENT"), var("SR_RECIPIENT_FILE")) {
            (Some(r), _) => r,
            (None, Some(f)) => std::fs::read_to_string(&f)
                .with_context(|| format!("reading SR_RECIPIENT_FILE {f}"))?
                .lines()
                .map(str::trim)
                .find(|l| l.starts_with("age1"))
                .context("no age1… line in SR_RECIPIENT_FILE")?
                .to_string(),
            (None, None) => anyhow::bail!(
                "SR_RECIPIENT is required (the age1… public key). Generate a key pair OFFLINE with `sr keygen`."
            ),
        };
        if recipient.contains("AGE-SECRET-KEY") {
            anyhow::bail!("SR_RECIPIENT contains a SECRET key. The server must only ever get the public age1… recipient.");
        }
        crate::seal::parse_recipient(&recipient).context("SR_RECIPIENT")?;

        Ok(Config {
            bind: var("SR_BIND").unwrap_or_else(|| "0.0.0.0:8080".into()),
            data_dir: var("SR_DATA_DIR").unwrap_or_else(|| "/data".into()).into(),
            name: var("SR_NAME").unwrap_or_else(|| "Témoin".into()),
            public_url: var("SR_PUBLIC_URL").map(|u| u.trim_end_matches('/').to_string()),
            recipient,
            admin_token: var("SR_ADMIN_TOKEN"),
            admin_path: {
                let p = var("SR_ADMIN_PATH").unwrap_or_else(|| "/admin".into());
                let p = if p.starts_with('/') { p } else { format!("/{p}") };
                if p.len() < 2 || !p[1..].bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
                    anyhow::bail!("SR_ADMIN_PATH must look like /some-secret-path (letters, digits, - and _)");
                }
                p
            },
            stealth: parse("SR_STEALTH", false)?,
            request_timeout_secs: parse("SR_REQUEST_TIMEOUT_SECS", 300)?,
            accepting_default: parse("SR_ACCEPTING", true)?,
            max_object_bytes: parse("SR_MAX_OBJECT_BYTES", 16 * 1024 * 1024)?,
            device_quota_bytes: parse("SR_DEVICE_QUOTA_BYTES", 20 * GIB)?,
            max_total_bytes: var("SR_MAX_TOTAL_BYTES").map(|v| v.parse()).transpose()?,
            min_free_bytes: parse("SR_MIN_FREE_BYTES", 50 * GIB)?,
            max_video_height: parse("SR_MAX_VIDEO_HEIGHT", 1080)?,
            default_video_height: parse("SR_DEFAULT_VIDEO_HEIGHT", 720)?,
            video_bitrate_720: parse("SR_VIDEO_BITRATE_720", 2_500_000)?,
            video_bitrate_1080: parse("SR_VIDEO_BITRATE_1080", 5_000_000)?,
            photo_max_pixels: parse("SR_PHOTO_MAX_PIXELS", 4_000_000)?,
            trust_proxy: parse("SR_TRUST_PROXY", false)?,
            log_ips: parse("SR_LOG_IPS", true)?,
            tunnel: match var("SR_TUNNEL").as_deref().map(str::to_ascii_lowercase).as_deref() {
                None | Some("off") | Some("none") | Some("false") => "off".into(),
                Some("cloudflare") | Some("cloudflared") => "cloudflare".into(),
                Some(other) => anyhow::bail!("SR_TUNNEL={other}: expected off or cloudflare"),
            },
            cloudflared: var("SR_CLOUDFLARED").unwrap_or_else(|| "cloudflared".into()).into(),
            acme_domain: var("SR_ACME_DOMAIN"),
            acme_email: var("SR_ACME_EMAIL"),
            https_bind: var("SR_HTTPS_BIND").unwrap_or_else(|| "0.0.0.0:443".into()),
            acme_staging: parse("SR_ACME_STAGING", false)?,
        })
    }

    /// Production defaults (same as an empty environment) for an embedding program such
    /// as the desktop app, which then adjusts what it needs.
    pub fn with_defaults(data_dir: PathBuf, recipient: String) -> Self {
        Config {
            bind: "127.0.0.1:8080".into(),
            data_dir,
            name: "Témoin".into(),
            public_url: None,
            recipient,
            admin_token: None,
            admin_path: "/admin".into(),
            stealth: false,
            request_timeout_secs: 300,
            accepting_default: true,
            max_object_bytes: 16 * 1024 * 1024,
            device_quota_bytes: 20 * GIB,
            max_total_bytes: None,
            min_free_bytes: 5 * GIB,
            max_video_height: 1080,
            default_video_height: 720,
            video_bitrate_720: 2_500_000,
            video_bitrate_1080: 5_000_000,
            photo_max_pixels: 4_000_000,
            trust_proxy: false,
            log_ips: false,
            tunnel: "off".into(),
            cloudflared: "cloudflared".into(),
            acme_domain: None,
            acme_email: None,
            https_bind: "0.0.0.0:443".into(),
            acme_staging: false,
        }
    }

    /// A config suitable for tests.
    pub fn for_tests(data_dir: PathBuf, recipient: String) -> Self {
        Config {
            bind: "127.0.0.1:0".into(),
            data_dir,
            name: "test".into(),
            public_url: Some("https://record.example.test".into()),
            recipient,
            admin_token: Some("admin-secret".into()),
            admin_path: "/admin".into(),
            stealth: false,
            request_timeout_secs: 300,
            accepting_default: true,
            max_object_bytes: 4 * 1024 * 1024,
            device_quota_bytes: 10 * 1024 * 1024,
            max_total_bytes: None,
            min_free_bytes: 0,
            max_video_height: 1080,
            default_video_height: 720,
            video_bitrate_720: 2_500_000,
            video_bitrate_1080: 5_000_000,
            photo_max_pixels: 4_000_000,
            trust_proxy: true,
            log_ips: true,
            tunnel: "off".into(),
            cloudflared: "cloudflared".into(),
            acme_domain: None,
            acme_email: None,
            https_bind: "127.0.0.1:0".into(),
            acme_staging: true,
        }
    }
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn parse<T: std::str::FromStr>(name: &str, default: T) -> anyhow::Result<T>
where
    T::Err: std::fmt::Display,
{
    match var(name) {
        None => Ok(default),
        Some(v) => v.parse().map_err(|e| anyhow::anyhow!("{name}={v}: {e}")),
    }
}
