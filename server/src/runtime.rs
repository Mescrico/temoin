//! Runs a server: HTTP listener, optional built-in HTTPS (Let's Encrypt), optional quick
//! tunnel, background sweeper. Shared by the `temoin-server` binary and the
//! desktop app.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use crate::server::{router, AppState};
use crate::tunnel::TunnelStatus;

/// Serves until `shutdown` resolves, then stops every background task.
pub async fn serve(state: Arc<AppState>, shutdown: impl Future<Output = ()> + Send + 'static) -> anyhow::Result<()> {
    let bind: SocketAddr = state.cfg.bind.parse()?;
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let local = listener.local_addr()?;
    tracing::info!("listening on http://{local}");
    let mut tasks = Vec::new();

    // Hand-back links: expire unopened ones, close download windows, archive their
    // content and move retrieved recordings to returned/ (nothing is deleted).
    let st = state.clone();
    tasks.push(tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(300));
        loop {
            tick.tick().await;
            let st = st.clone();
            let swept = tokio::task::spawn_blocking(move || {
                let n = st.shares.sweep();
                st.process_returns();
                n
            });
            if let Ok(n) = swept.await {
                if n > 0 {
                    tracing::info!(archived = n, "hand-back links swept");
                }
            }
        }
    }));

    if state.cfg.tunnel == "cloudflare" {
        let target = format!("http://127.0.0.1:{}", local.port());
        tasks.push(tokio::spawn(crate::tunnel::run_cloudflared(state.clone(), state.cfg.cloudflared.clone(), target)));
    }

    let https = match state.cfg.acme_domain.clone() {
        Some(domain) => Some(spawn_acme(state.clone(), domain)?),
        None => None,
    };

    let app = router(state.clone());
    let result = axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown)
        .await;
    if let Some(handle) = https {
        handle.graceful_shutdown(Some(Duration::from_secs(5)));
    }
    for t in tasks {
        t.abort();
    }
    result?;
    Ok(())
}

/// Built-in HTTPS on `https_bind` with a Let's Encrypt certificate (TLS-ALPN-01: only
/// port 443 has to reach this machine). Certificates are cached in `<data>/acme`.
fn spawn_acme(state: Arc<AppState>, domain: String) -> anyhow::Result<axum_server::Handle<SocketAddr>> {
    use rustls_acme::caches::DirCache;
    use rustls_acme::AcmeConfig;
    use tokio_stream::StreamExt;

    let addr: SocketAddr = state.cfg.https_bind.parse()?;
    let mut acme = AcmeConfig::new([domain.clone()])
        .contact(state.cfg.acme_email.iter().map(|e| format!("mailto:{e}")))
        .cache(DirCache::new(state.cfg.data_dir.join("acme")))
        .directory_lets_encrypt(!state.cfg.acme_staging)
        .state();
    let acceptor = acme.axum_acceptor(acme.default_rustls_config());
    let url = format!("https://{domain}{}", if addr.port() == 443 { String::new() } else { format!(":{}", addr.port()) });
    if state.cfg.public_url.is_none() {
        state.set_public_url(Some(url.clone()));
    }
    state.set_tunnel(TunnelStatus { mode: "acme".into(), state: "starting".into(), url: Some(url.clone()), error: None });

    let st = state.clone();
    tokio::spawn(async move {
        while let Some(event) = acme.next().await {
            match event {
                Ok(ok) => {
                    tracing::info!("acme: {ok:?}");
                    st.set_tunnel(TunnelStatus { mode: "acme".into(), state: "up".into(), url: Some(url.clone()), error: None });
                }
                Err(err) => {
                    tracing::error!("acme: {err:?}");
                    st.set_tunnel(TunnelStatus { mode: "acme".into(), state: "down".into(), url: Some(url.clone()), error: Some(format!("{err}")) });
                }
            }
        }
    });

    let handle = axum_server::Handle::new();
    let app = router(state);
    let h = handle.clone();
    tokio::spawn(async move {
        tracing::info!("listening on https://{addr} for {domain}");
        if let Err(e) = axum_server::bind(addr)
            .handle(h)
            .acceptor(acceptor)
            .serve(app.into_make_service_with_connect_info::<SocketAddr>())
            .await
        {
            tracing::error!("https listener: {e}");
        }
    });
    Ok(handle)
}
