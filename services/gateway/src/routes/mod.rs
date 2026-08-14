//! REST surface. One module per backend namespace, mirroring the
//! `atlas.auth` / `atlas.geo` / `atlas.payments` split in the SDK.
//!
//! Route naming follows the platform's public vocabulary rather than the
//! gRPC method names, because this is the surface the TypeScript, Dart,
//! and Rust SDKs will wrap.

pub mod auth;
pub mod geo;
pub mod ops;
pub mod payments;

use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::Method;
use axum::{middleware, Router};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::metrics;
use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    // Any-origin CORS is correct for a token-authenticated public API:
    // credentials never ride in cookies, so there is no CSRF surface to
    // protect, and browser SDK users can call from any origin. Note that
    // `allow_credentials` must stay off — the CORS spec forbids pairing
    // it with a wildcard origin, and turning it on would be the change
    // that introduces the CSRF surface.
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::DELETE])
        .allow_headers([AUTHORIZATION, CONTENT_TYPE]);

    Router::new()
        .merge(ops::routes())
        .nest("/v1/auth", auth::routes())
        .nest("/v1/geo", geo::routes())
        .nest("/v1/payments", payments::routes())
        // Layer order is outermost-first. Metrics wraps everything so a
        // panic-turned-500 or a rejected body still gets counted.
        .layer(middleware::from_fn(metrics::track))
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}
