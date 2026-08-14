//! Atlas control-plane entrypoint.
//!
//! Boot order:
//!   1. Install tracing + Prometheus recorder.
//!   2. Connect to Postgres — fail fast, since no route works without it.
//!   3. Spawn the metrics HTTP server on its own port.
//!   4. Serve the REST API on 8081, matching the CLI's default base URL.
//!   5. Trap SIGTERM / SIGINT for graceful shutdown.

use anyhow::Context;
use atlas_control_plane::{config, db, metrics, routes, state};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cfg = config::Config::from_env();
    info!(
        http_addr = %cfg.http_addr,
        metrics_addr = %cfg.metrics_addr,
        gateway_metrics = %cfg.gateway_metrics_url,
        "starting atlas-control-plane"
    );

    let metrics_handle = metrics::install_recorder();
    metrics::serve(cfg.metrics_addr, metrics_handle).await?;

    // Unlike the gateway's lazy upstream channels, this connects eagerly:
    // every route touches Postgres, so a control plane that cannot reach
    // it has nothing to serve and should fail loudly at boot.
    let pool = db::connect(&cfg.database_url, cfg.database_pool_size)
        .await
        .context("postgres connection")?;
    info!("postgres pool ready");

    let http_addr = cfg.http_addr;
    let app = routes::router(state::AppState::new(pool, cfg)?);

    let listener = tokio::net::TcpListener::bind(http_addr)
        .await
        .with_context(|| format!("binding {http_addr}"))?;
    info!(addr = %http_addr, "control plane listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("axum server")?;

    info!("atlas-control-plane exited cleanly");
    Ok(())
}

async fn shutdown_signal() {
    use tokio::signal;
    let ctrl_c = async {
        signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    info!("shutdown signal received");
}
