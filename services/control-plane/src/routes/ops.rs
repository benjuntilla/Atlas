//! Liveness and readiness.
//!
//! Unlike the gateway — which is stateless and stays ready even when its
//! upstreams are down — the control plane genuinely cannot serve a single
//! route without Postgres. Every endpoint reads or writes the `control`
//! schema. So readiness here really does check the database, and failing
//! it is correct: a replica that cannot reach Postgres has nothing to
//! offer and should be pulled from the load balancer.

use axum::extract::State;
use axum::http::StatusCode;
use axum::{routing::get, Json, Router};
use serde_json::{json, Value};

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
}

/// Liveness: the process is up. Deliberately does not touch the database —
/// a liveness probe that fails on a transient database blip would get the
/// container killed instead of letting it recover.
async fn healthz() -> (StatusCode, Json<Value>) {
    (StatusCode::OK, Json(json!({ "status": "ok" })))
}

async fn readyz(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => (
            StatusCode::OK,
            Json(json!({ "status": "ready", "checks": { "database": "ok" } })),
        ),
        Err(e) => {
            tracing::warn!(error = %e, "readiness check failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "status": "not_ready", "checks": { "database": "unreachable" } })),
            )
        }
    }
}
