use srec::config::Config;
use srec::server::{AppState, VERSION};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_env("SR_LOG").unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let cfg = Config::from_env()?;
    tracing::info!(
        version = VERSION,
        data = %cfg.data_dir.display(),
        recipient = %cfg.recipient,
        fingerprint = %srec::seal::fingerprint(&cfg.recipient),
        admin = cfg.admin_token.is_some(),
        "starting temoin-server (write-only)"
    );
    if std::env::var("SR_UPLOAD_TOKEN").is_ok() {
        tracing::warn!("SR_UPLOAD_TOKEN is obsolete since 0.3.0: phones now join with invitations (see /admin)");
    }
    let state = AppState::open(cfg)?;
    srec::runtime::serve(state, async {
        let ctrl_c = tokio::signal::ctrl_c();
        #[cfg(unix)]
        {
            let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
            tokio::select! { _ = ctrl_c => {}, _ = term.recv() => {} }
        }
        #[cfg(not(unix))]
        {
            let _ = ctrl_c.await;
        }
        tracing::info!("shutting down");
    })
    .await?;
    Ok(())
}
