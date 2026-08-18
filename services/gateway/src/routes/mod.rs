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

use axum::http::header::{HeaderName, AUTHORIZATION, CONTENT_TYPE};
use axum::http::Method;
use axum::{middleware, Router};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::metrics;
use crate::ratelimit::{self, Limiters};
use crate::state::AppState;
use crate::tenant::{self, Tenant};
use std::sync::Arc;

/// How a request's project is determined.
///
/// Only [`TenantSource::Key`] is used by [`router`], the constructor
/// `main.rs` calls, so it is the only variant that runs in production.
#[derive(Clone)]
pub enum TenantSource {
    /// Resolve the `X-Atlas-Key` header against `control.api_keys`.
    Key,
    /// Use this project for every request without resolving anything.
    /// Test-only; see [`crate::tenant::fixed_layer`].
    Fixed(Box<Tenant>),
}

/// Router for production, where the listener supplies a peer address.
///
/// Rate limiting sits OUTSIDE the metrics layer so a throttled request is
/// still counted — a spike of 429s is exactly what you want to see on a
/// dashboard, and a limiter that hides its own effect is untuneable.
pub fn router(state: AppState, limiters: Arc<Limiters>) -> Router {
    base_router(state, TenantSource::Key).layer(middleware::from_fn_with_state(
        Arc::clone(&limiters),
        ratelimit::layer,
    ))
}

/// Router for tests and for any listener without `ConnectInfo`. Identical
/// except that every request shares the "unknown" address bucket.
pub fn router_without_peer(
    state: AppState,
    limiters: Arc<Limiters>,
    tenants: TenantSource,
) -> Router {
    base_router(state, tenants).layer(middleware::from_fn_with_state(
        Arc::clone(&limiters),
        ratelimit::layer_without_peer,
    ))
}

fn base_router(state: AppState, tenants: TenantSource) -> Router {
    // Any-origin CORS is correct for a token-authenticated public API:
    // credentials never ride in cookies, so there is no CSRF surface to
    // protect, and browser SDK users can call from any origin. Note that
    // `allow_credentials` must stay off — the CORS spec forbids pairing
    // it with a wildcard origin, and turning it on would be the change
    // that introduces the CSRF surface.
    // `x-atlas-key` must be listed or every browser request preflights and
    // fails: a custom header is not CORS-safelisted, so omitting it here
    // would make the whole API unreachable from a browser SDK while
    // working fine from curl.
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::DELETE])
        .allow_headers([
            AUTHORIZATION,
            CONTENT_TYPE,
            HeaderName::from_static(tenant::KEY_HEADER),
        ]);

    // The tenant layer wraps the three `/v1` sub-routers as a group rather
    // than being applied per handler. That is the point: a route added to
    // any of them later is tenant-checked without anyone remembering to
    // check it. `ops::routes()` (health, readiness) sits outside, because
    // a probe carries no customer key and must answer whether or not
    // Postgres is reachable.
    //
    // It sits INSIDE the rate limiter (applied by the callers above), so a
    // flood of invalid keys is throttled before it can become a flood of
    // database lookups.
    let v1 = Router::new()
        .nest("/auth", auth::routes())
        .nest("/geo", geo::routes())
        .nest("/payments", payments::routes());
    let v1 = match tenants {
        TenantSource::Key => v1.layer(middleware::from_fn_with_state(state.clone(), tenant::layer)),
        TenantSource::Fixed(t) => v1.layer(middleware::from_fn_with_state(*t, tenant::fixed_layer)),
    };

    Router::new()
        .merge(ops::routes())
        .nest("/v1", v1)
        // Layer order is outermost-first. Metrics wraps everything so a
        // panic-turned-500 or a rejected body still gets counted.
        .layer(middleware::from_fn(metrics::track))
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}
