//! Prometheus metrics: recorder install, scrape endpoint, and the
//! per-request tracking middleware.
//!
//! Mirrors the geo-engine setup so both Rust services expose the same
//! shape of data. The scrape endpoint binds its own listener on
//! `METRICS_ADDR` rather than living on the public router — `/metrics`
//! on a public edge would hand out request volumes and error rates to
//! anyone who asked.

use axum::extract::{MatchedPath, Request};
use axum::middleware::Next;
use axum::response::Response;
use axum::{routing::get, Router};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::net::SocketAddr;
use std::time::Instant;
use tracing::info;

pub fn install_recorder() -> PrometheusHandle {
    PrometheusBuilder::new()
        .install_recorder()
        .expect("failed to install prometheus recorder")
}

/// Spawn the `/metrics` server. Returns once the listener is bound.
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

/// Record request count and latency, labelled by route template.
///
/// The label is the *matched path* (`/v1/geo/geofences/:id`), never the
/// concrete URI (`/v1/geo/geofences/9f3c...`). Using the raw path would
/// mint a new time series per geofence id and melt Prometheus. Unmatched
/// requests (404s) collapse to a single `unmatched` bucket for the same
/// reason.
pub async fn track(req: Request, next: Next) -> Response {
    let path = req
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_else(|| "unmatched".to_string());
    let method = req.method().clone();

    let start = Instant::now();
    let response = next.run(req).await;
    let latency_ms = start.elapsed().as_secs_f64() * 1000.0;

    let status = response.status().as_u16().to_string();
    metrics::counter!(
        "atlas_gateway_requests_total",
        "method" => method.to_string(),
        "path" => path.clone(),
        "status" => status,
    )
    .increment(1);
    metrics::histogram!(
        "atlas_gateway_request_duration_ms",
        "method" => method.to_string(),
        "path" => path,
    )
    .record(latency_ms);

    response
}
