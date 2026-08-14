//! Router-level tests that need no database.
//!
//! The pool is built with `connect_lazy` against a port nothing listens
//! on, so any handler that reaches Postgres fails loudly rather than
//! quietly passing. That makes these tests a precise statement about what
//! the control plane decides *before* touching state:
//!
//!   * unauthenticated requests are rejected at the extractor
//!   * a malformed key is rejected on shape alone, without a query
//!
//! Everything that needs real rows lives in `db_test.rs`, behind
//! `#[ignore]`.

use atlas_control_plane::{config::Config, routes, state::AppState};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

fn test_config() -> Config {
    Config {
        http_addr: "127.0.0.1:0".parse().unwrap(),
        metrics_addr: "127.0.0.1:0".parse().unwrap(),
        // Port nothing listens on. If a test here ever reaches the
        // database it will fail, which is the intent.
        database_url: "postgres://atlas:atlas_dev@127.0.0.1:59431/atlas".to_string(),
        database_pool_size: 1,
        auth_addr: "http://127.0.0.1:59432".to_string(),
        geo_addr: "http://127.0.0.1:59433".to_string(),
        payments_addr: "http://127.0.0.1:59434".to_string(),
        kafka_brokers: "127.0.0.1:59435".to_string(),
        gateway_metrics_url: "http://127.0.0.1:59436/metrics".to_string(),
        endpoint_template: "https://api.atlas.dev/v1/{name}".to_string(),
        probe_timeout: std::time::Duration::from_millis(150),
    }
}

fn app() -> axum::Router {
    let cfg = test_config();
    let pool = PgPoolOptions::new()
        .max_connections(1)
        // sqlx defaults to a 30s acquire timeout, which is how long the
        // readiness test would sit waiting on a port nothing listens on.
        // These tests want the failure, not the wait.
        .acquire_timeout(std::time::Duration::from_millis(300))
        // Lazy: no connection is attempted until a query runs.
        .connect_lazy(&cfg.database_url)
        .expect("lazy pool always builds");
    routes::router(AppState::new(pool, cfg).expect("state builds"))
}

async fn status_of(req: Request<Body>) -> StatusCode {
    app().oneshot(req).await.expect("router responds").status()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

fn get_with_key(uri: &str, key: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header("authorization", format!("Bearer {key}"))
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn liveness_needs_no_credentials_and_no_database() {
    assert_eq!(status_of(get("/healthz")).await, StatusCode::OK);
}

/// Readiness must fail when Postgres is unreachable — unlike the gateway,
/// this service can serve nothing without it.
#[tokio::test]
async fn readiness_fails_without_a_database() {
    assert_eq!(
        status_of(get("/readyz")).await,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn every_project_route_requires_a_key() {
    let routes = [
        "/v1/projects/demo/status",
        "/v1/projects/demo/logs",
        "/v1/projects/demo/keys",
    ];
    for uri in routes {
        assert_eq!(
            status_of(get(uri)).await,
            StatusCode::UNAUTHORIZED,
            "{uri} must require authentication"
        );
    }
}

#[tokio::test]
async fn deploy_requires_a_key() {
    let req = Request::builder()
        .method("POST")
        .uri("/v1/projects")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"name":"demo","region":"us-central1","environment":"production","services_enabled":["auth"]}"#))
        .unwrap();
    assert_eq!(status_of(req).await, StatusCode::UNAUTHORIZED);
}

/// The shape pre-filter in `keys::looks_like_key` exists so junk tokens
/// cost nothing. With the database unreachable, a token that got as far as
/// a query would surface as a 500 — so a 401 here proves the filter ran
/// first.
#[tokio::test]
async fn malformed_keys_are_rejected_without_a_query() {
    let junk = [
        "not-a-key",
        // Another scheme's key shape. Not a real vendor prefix on purpose:
        // secret scanners flag those even in fixtures.
        "zzz_live_0123456789abcdef0123456789ab",
        "atl_live_tooshort",
        "",
    ];
    for token in junk {
        let code = status_of(get_with_key("/v1/projects/demo/status", token)).await;
        assert_eq!(
            code,
            StatusCode::UNAUTHORIZED,
            "token {token:?} should be rejected on shape, got {code}"
        );
    }
}

#[tokio::test]
async fn wrong_auth_scheme_is_rejected() {
    for header in [
        "Basic abcdef",
        "atl_live_0123456789abcdef01234567",
        "Bearer ",
    ] {
        let req = Request::builder()
            .uri("/v1/projects/demo/status")
            .header("authorization", header)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            status_of(req).await,
            StatusCode::UNAUTHORIZED,
            "header {header:?} must not authenticate"
        );
    }
}

/// Error bodies are printed straight to a terminal by the CLI, so the
/// envelope shape is part of the contract.
#[tokio::test]
async fn errors_use_the_documented_envelope() {
    let response = app()
        .oneshot(get("/v1/projects/demo/status"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["error"]["code"], "unauthenticated");
    assert!(json["error"]["message"].is_string());
}

/// The CLI builds URLs as `{base_url}/projects/...` with base_url ending
/// in `/v1`. A route mounted anywhere else is invisible to it.
#[tokio::test]
async fn routes_are_mounted_under_v1() {
    // Unversioned paths do not exist.
    assert_eq!(
        status_of(get("/projects/demo/status")).await,
        StatusCode::NOT_FOUND
    );
    // The versioned one exists and gets as far as authentication.
    assert_eq!(
        status_of(get("/v1/projects/demo/status")).await,
        StatusCode::UNAUTHORIZED
    );
}
