//! Reachability for a server running on a home PC.
//!
//! **Quick tunnel** (`SR_TUNNEL=cloudflare`): runs Cloudflare's `cloudflared` in
//! "quick tunnel" mode. No account, no router setup: it prints a random
//! `https://….trycloudflare.com` address that phones can reach from anywhere, and the
//! server uses it as its public URL (invitation QR codes follow it). Cloudflare sees
//! connection metadata (IP, sizes, times), never content: objects are age-encrypted on
//! the phone. The address changes when cloudflared restarts; phones then re-scan an
//! invitation and keep their identity (same server key).

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::server::AppState;

#[derive(Debug, Clone, Serialize, Default)]
pub struct TunnelStatus {
    /// "off", "cloudflare", "acme"
    pub mode: String,
    /// "off", "starting", "up", "down"
    pub state: String,
    pub url: Option<String>,
    pub error: Option<String>,
}

/// The trycloudflare address in a line of cloudflared output, if any.
pub fn find_quick_url(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let url: String = line[start..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '/' | '.' | '-'))
        .collect();
    let host = url.trim_start_matches("https://");
    (host.ends_with(".trycloudflare.com") && host != "api.trycloudflare.com").then_some(url)
}

/// Keeps a cloudflared quick tunnel to `local` (e.g. `http://127.0.0.1:8080`) running for
/// as long as the server runs, restarting it if it dies.
pub async fn run_cloudflared(st: Arc<AppState>, bin: PathBuf, local: String) {
    let mut wait = 3u64;
    loop {
        st.set_tunnel(TunnelStatus { mode: "cloudflare".into(), state: "starting".into(), url: None, error: None });
        let mut cmd = tokio::process::Command::new(&bin);
        cmd.args(["tunnel", "--no-autoupdate", "--url", &local])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let msg = format!("cannot start {}: {e}", bin.display());
                tracing::error!("{msg}");
                st.set_tunnel(TunnelStatus { mode: "cloudflare".into(), state: "down".into(), url: None, error: Some(msg) });
                tokio::time::sleep(Duration::from_secs(30)).await;
                continue;
            }
        };
        let stderr = child.stderr.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let st2 = st.clone();
        let reader = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            let mut out = BufReader::new(stdout).lines();
            let mut last_error = None;
            loop {
                let line = tokio::select! {
                    l = lines.next_line() => l,
                    l = out.next_line() => l,
                };
                let Ok(Some(line)) = line else { break };
                tracing::debug!(target: "cloudflared", "{line}");
                if let Some(url) = find_quick_url(&line) {
                    tracing::info!(%url, "quick tunnel up");
                    st2.set_public_url(Some(url.clone()));
                    st2.set_tunnel(TunnelStatus { mode: "cloudflare".into(), state: "up".into(), url: Some(url), error: None });
                } else if line.contains(" ERR ") {
                    last_error = Some(line.split(" ERR ").nth(1).unwrap_or(&line).trim().chars().take(200).collect::<String>());
                }
            }
            last_error
        });
        let status = child.wait().await;
        let last_error = reader.await.ok().flatten();
        let msg = format!("cloudflared stopped ({}){}", status.map(|s| s.to_string()).unwrap_or_else(|e| e.to_string()),
            last_error.map(|e| format!(": {e}")).unwrap_or_default());
        tracing::warn!("{msg}");
        st.set_public_url(st.cfg.public_url.clone());
        st.set_tunnel(TunnelStatus { mode: "cloudflare".into(), state: "down".into(), url: None, error: Some(msg) });
        tokio::time::sleep(Duration::from_secs(wait)).await;
        wait = (wait * 2).min(60);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_quick_tunnel_url() {
        let l = "2026-10-20T10:00:00Z INF |  https://brave-otter-sun-river.trycloudflare.com                        |";
        assert_eq!(find_quick_url(l).as_deref(), Some("https://brave-otter-sun-river.trycloudflare.com"));
        assert_eq!(find_quick_url("INF Requesting new quick Tunnel on trycloudflare.com..."), None);
        assert_eq!(find_quick_url("ERR https://api.trycloudflare.com/tunnel failed"), None);
        assert_eq!(find_quick_url("see https://developers.cloudflare.com/x"), None);
    }
}
