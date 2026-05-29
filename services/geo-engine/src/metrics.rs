//! Prometheus metrics setup.
//!
//! The recorder is installed once at process start. After that, calls to
//! `metrics::histogram!` and `metrics::counter!` from anywhere in the
//! crate are recorded and exposed by the HTTP scrape endpoint.
//!
//! Histogram names ending in `_ms` are in milliseconds; `_seconds` is
//! Prometheus convention but we keep tighter buckets in ms for query
//! latency where sub-second granularity matters.

use axum::{routing::get, Router};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::net::SocketAddr;
use tracing::info;

/// Install the global recorder and return a handle the HTTP scrape
/// endpoint will render.
pub fn install_recorder() -> PrometheusHandle {
    PrometheusBuilder::new()
        .install_recorder()
        .expect("failed to install prometheus recorder")
}

/// Spawn the /metrics HTTP server. Returns once the listener is bound.
pub async fn serve(addr: SocketAddr, handle: PrometheusHandle) -> anyhow::Result<()> {
    let app = Router::new().route(
        "/metrics",
        get(move || {
            let h = handle.clone();
            async move { h.render() }
        }),
    );
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, "metrics exporter listening");
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(error = %e, "metrics server crashed");
        }
    });
    Ok(())
}
