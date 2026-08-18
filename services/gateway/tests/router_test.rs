//! Router-level tests driven through `tower::ServiceExt::oneshot` — no
//! port binding, no backends.
//!
//! These specifically cover what the gateway decides *before* it talks to
//! anything, which is exactly the security-relevant part:
//!
//!   * an unauthenticated request is rejected at the extractor, so a
//!     handler never runs and no upstream call is made
//!   * authentication is checked before the request body is parsed, so a
//!     garbage body cannot turn a 401 into a 400 (or vice versa)
//!   * the money-moving RPCs that cannot be authorized are simply not
//!     routed
//!
//! Upstreams point at ports nothing listens on. That is deliberate: if a
//! test expecting 401 ever reaches the network, it fails with 503 instead
//! of silently passing.

use atlas_gateway::{
    config::Config,
    ratelimit::{Limiters, RateLimitConfig},
    routes,
    state::AppState,
};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

/// Ports in the IANA ephemeral range that no Atlas service uses.
fn test_config() -> Config {
    Config {
        http_addr: "127.0.0.1:0".parse().unwrap(),
        metrics_addr: "127.0.0.1:0".parse().unwrap(),
        auth_addr: "http://127.0.0.1:59151".to_string(),
        geo_addr: "http://127.0.0.1:59152".to_string(),
        payments_addr: "http://127.0.0.1:59153".to_string(),
        upstream_timeout: std::time::Duration::from_millis(250),
        upstream_connect_timeout: std::time::Duration::from_millis(250),
        // Unused: these tests build limiters explicitly via app_with_limits.
        rate_limit: RateLimitConfig::default(),
    }
}

fn app() -> axum::Router {
    // Limits high enough that the functional tests below never hit them;
    // rate limiting has its own tests with deliberately tiny quotas.
    app_with_limits(RateLimitConfig {
        default_per_minute: 10_000,
        auth_per_minute: 10_000,
        ..RateLimitConfig::default()
    })
}

fn app_with_limits(limits: RateLimitConfig) -> axum::Router {
    routes::router_without_peer(
        AppState::connect(&test_config()).expect("lazy channels always build"),
        Limiters::new(limits),
    )
}

async fn status_of(req: Request<Body>) -> StatusCode {
    app().oneshot(req).await.expect("router responds").status()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("valid request")
}

fn post_json(uri: &str, body: &'static str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body))
        .expect("valid request")
}

#[tokio::test]
async fn healthz_needs_no_credentials() {
    assert_eq!(status_of(get("/healthz")).await, StatusCode::OK);
    assert_eq!(status_of(get("/readyz")).await, StatusCode::OK);
}

#[tokio::test]
async fn protected_routes_reject_missing_bearer() {
    // If any of these reached the network they would be 503, not 401.
    assert_eq!(
        status_of(get("/v1/auth/me")).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        status_of(get("/v1/geo/nearby?lat=0&lng=0&radius_m=100")).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        status_of(get("/v1/payments/wallet")).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        status_of(get("/v1/geo/geofences")).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn malformed_authorization_headers_are_rejected() {
    for header in [
        "not-a-scheme",
        "Basic dXNlcjpwYXNzd29yZA==",
        "Bearer ",
        "Bearer    ",
    ] {
        let req = Request::builder()
            .uri("/v1/auth/me")
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

/// Extractor ordering guarantee: `AuthUser` is declared before `Json` in
/// Deposits credit a wallet, so an unauthenticated one would be a way to
/// mint balance for an arbitrary user.
#[tokio::test]
async fn deposit_requires_authentication() {
    let req = post_json("/v1/payments/deposits", r#"{"amount_cents":1000}"#);
    assert_eq!(status_of(req).await, StatusCode::UNAUTHORIZED);
}

/// every protected handler, so an unauthenticated request with an invalid
/// body is a 401. If someone reorders the arguments this flips to 400 and
/// the body would be parsed for an anonymous caller.
#[tokio::test]
async fn auth_is_checked_before_the_body_is_parsed() {
    let req = post_json("/v1/geo/locations", "{ this is not json");
    assert_eq!(status_of(req).await, StatusCode::UNAUTHORIZED);

    let req = post_json("/v1/payments/transactions", "{ this is not json");
    assert_eq!(status_of(req).await, StatusCode::UNAUTHORIZED);
}

/// Public routes still validate their bodies. A blank login must not cost
/// a gRPC round trip — if this ever reaches the unreachable auth backend
/// it returns 503 and this assertion catches it.
#[tokio::test]
async fn public_routes_validate_input_before_fanout() {
    let req = post_json("/v1/auth/login", r#"{"email":"","password":""}"#);
    assert_eq!(status_of(req).await, StatusCode::BAD_REQUEST);

    let req = post_json(
        "/v1/auth/register",
        r#"{"email":"  ","password":"hunter2"}"#,
    );
    assert_eq!(status_of(req).await, StatusCode::BAD_REQUEST);
}

/// Settle and refund take a bare transaction_id with no ownership signal,
/// so they are intentionally unrouted. See `routes/payments.rs`. A 404
/// here means "no such route"; if someone adds one, this test fails and
/// forces a look at whether the authorization story changed.
#[tokio::test]
async fn unauthorizable_payment_rpcs_are_not_routed() {
    let tx = "00000000-0000-0000-0000-000000000000";
    let req = post_json("/v1/payments/transactions/settle", "{}");
    assert_eq!(status_of(req).await, StatusCode::NOT_FOUND);

    for uri in [
        format!("/v1/payments/transactions/{tx}/settle"),
        format!("/v1/payments/transactions/{tx}/refund"),
    ] {
        let req = Request::builder()
            .method("POST")
            .uri(&uri)
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();
        assert_eq!(status_of(req).await, StatusCode::NOT_FOUND, "{uri}");
    }
}

/// `auth.IssueToken` mints a token for an arbitrary user_id with no
/// credential check. It must never be reachable over HTTP.
#[tokio::test]
async fn issue_token_is_not_routed() {
    let req = post_json("/v1/auth/issue-token", r#"{"user_id":"x"}"#);
    assert_eq!(status_of(req).await, StatusCode::NOT_FOUND);

    let req = post_json("/v1/auth/token", r#"{"user_id":"x"}"#);
    assert_eq!(status_of(req).await, StatusCode::NOT_FOUND);
}

/// `payments.DrainOutbox` is an ops primitive, marked internal in the
/// proto.
#[tokio::test]
async fn drain_outbox_is_not_routed() {
    let req = post_json("/v1/payments/outbox/drain", "{}");
    assert_eq!(status_of(req).await, StatusCode::NOT_FOUND);
}

/// The error envelope is part of the public contract that SDKs branch on,
/// so its shape is asserted rather than assumed.
#[tokio::test]
async fn errors_use_the_documented_envelope() {
    let response = app()
        .oneshot(get("/v1/auth/me"))
        .await
        .expect("router responds");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("valid json body");

    assert_eq!(json["error"]["code"], "unauthenticated");
    assert!(
        json["error"]["message"].is_string(),
        "message must be a string, got {json}"
    );
}

// --- rate limiting ----------------------------------------------------------

/// Credential endpoints get a much smaller quota than everything else,
/// because they are the ones worth brute forcing.
#[tokio::test]
async fn login_is_throttled_after_the_credential_quota() {
    let app = app_with_limits(RateLimitConfig {
        auth_per_minute: 3,
        default_per_minute: 10_000,
        ..RateLimitConfig::default()
    });

    let body = r#"{"email":"a@b.dev","password":"x"}"#;
    // These fail on the unreachable backend, which is fine — what matters
    // is that the limiter counts them.
    for _ in 0..3 {
        let res = app
            .clone()
            .oneshot(post_json("/v1/auth/login", body))
            .await
            .unwrap();
        assert_ne!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    let res = app
        .clone()
        .oneshot(post_json("/v1/auth/login", body))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    // A client needs to know how long to wait.
    assert!(res.headers().contains_key("retry-after"));
}

/// Spending the credential quota must not lock a caller out of the rest of
/// the API — otherwise a few bad password attempts would take down a
/// logged-in session.
#[tokio::test]
async fn the_credential_quota_does_not_affect_other_routes() {
    let app = app_with_limits(RateLimitConfig {
        auth_per_minute: 1,
        default_per_minute: 10_000,
        ..RateLimitConfig::default()
    });

    let body = r#"{"email":"a@b.dev","password":"x"}"#;
    let _ = app.clone().oneshot(post_json("/v1/auth/login", body)).await;
    let throttled = app
        .clone()
        .oneshot(post_json("/v1/auth/login", body))
        .await
        .unwrap();
    assert_eq!(throttled.status(), StatusCode::TOO_MANY_REQUESTS);

    // Health and ordinary routes still answer.
    let health = app.clone().oneshot(get("/healthz")).await.unwrap();
    assert_eq!(health.status(), StatusCode::OK);
    let protected = app.clone().oneshot(get("/v1/auth/me")).await.unwrap();
    assert_eq!(protected.status(), StatusCode::UNAUTHORIZED);
}

/// The 429 body must use the same envelope as every other error, so an SDK
/// branching on `code` needs no special case.
#[tokio::test]
async fn a_throttled_response_uses_the_documented_envelope() {
    let app = app_with_limits(RateLimitConfig {
        auth_per_minute: 1,
        default_per_minute: 10_000,
        ..RateLimitConfig::default()
    });
    let body = r#"{"email":"a@b.dev","password":"x"}"#;
    let _ = app.clone().oneshot(post_json("/v1/auth/login", body)).await;

    let res = app
        .clone()
        .oneshot(post_json("/v1/auth/login", body))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);

    let bytes = axum::body::to_bytes(res.into_body(), 64 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["error"]["code"], "resource_exhausted");
    assert!(json["error"]["message"].is_string());
}

/// Health checks must never be throttled: a limiter that 429s the liveness
/// probe would have Kubernetes restart the pod under load, turning a busy
/// replica into a crash loop.
#[tokio::test]
async fn health_probes_survive_an_exhausted_default_quota() {
    let app = app_with_limits(RateLimitConfig {
        default_per_minute: 1,
        auth_per_minute: 10_000,
        ..RateLimitConfig::default()
    });

    // Spend the default quota on the shared "unknown" bucket.
    let _ = app.clone().oneshot(get("/healthz")).await;
    let res = app.clone().oneshot(get("/healthz")).await.unwrap();
    assert_eq!(
        res.status(),
        StatusCode::OK,
        "liveness must not be rate limited"
    );
}
