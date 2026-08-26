//! Liveness and readiness probes.
//!
//! # Why `/readyz` does not check the backends
//!
//! The tempting design is for readiness to fail when auth/geo/payments
//! are unreachable. That would be wrong here. The gateway is stateless
//! and holds no warm-up state, so there is nothing for readiness to wait
//! on. If every replica reported NOT READY whenever a backend was down,
//! Kubernetes would pull all of them out of the load balancer and callers
//! would get connection refused instead of a `503` with a JSON body
//! naming the failing service. Losing that diagnostic is a downgrade.
//!
//! So readiness asserts exactly one thing — this process can accept and
//! route HTTP — and per-request upstream failures surface as `503`
//! through the normal error path. Backend health is observable through
//! each backend's own gRPC health service and the
//! `atlas_gateway_errors_total{code="unavailable"}` counter.

use axum::http::StatusCode;
use axum::{routing::get, Json, Router};
use serde_json::{json, Value};

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
}

/// Liveness: the process is running and the async runtime is scheduling.
async fn healthz() -> (StatusCode, Json<Value>) {
    (StatusCode::OK, Json(json!({ "status": "ok" })))
}

/// Readiness: this replica can serve traffic. See the module note for why
/// this does not depend on backend availability.
async fn readyz() -> (StatusCode, Json<Value>) {
    (
        StatusCode::OK,
        Json(json!({
            "status": "ready",
            "checks": { "http_router": "ok" },
        })),
    )
}
