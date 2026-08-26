//! HTTP error type and the gRPC-to-HTTP status translation.
//!
//! Every failure the gateway can produce lands in [`ApiError`], which
//! renders a stable JSON envelope:
//!
//! ```json
//! { "error": { "code": "invalid_argument", "message": "radius_m must be > 0" } }
//! ```
//!
//! # What gets forwarded to the caller
//!
//! Upstream messages for *client* errors (4xx) are developer-facing and
//! actionable — "radius_m must be > 0 and <= 50000" is exactly what an
//! SDK user needs. Those pass through.
//!
//! Messages for *server* errors (5xx) do not. `Status::internal` strings
//! from a backend can carry table names, constraint names, or connection
//! strings; the geo-engine and payments handlers already scrub most of
//! this, but the gateway is the last line and does not rely on upstream
//! discipline. 5xx responses always carry a fixed generic message, with
//! the real one logged server-side.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use tonic::Code;
use tracing::{error, warn};

#[derive(Debug)]
pub enum ApiError {
    /// Rejected inside the gateway before any upstream call.
    BadRequest(String),
    /// Missing, malformed, or rejected credentials.
    Unauthorized(String),
    /// An upstream gRPC call returned a non-OK status.
    Upstream {
        service: &'static str,
        status: Box<tonic::Status>,
    },
}

impl ApiError {
    pub fn upstream(service: &'static str, status: tonic::Status) -> Self {
        ApiError::Upstream {
            service,
            status: Box::new(status),
        }
    }
}

/// Map a gRPC code onto the closest HTTP status.
///
/// `FailedPrecondition` maps to 422 rather than 409: the backends use it
/// for "your request was well-formed but the world is not in a state
/// where it can succeed" (insufficient funds, wrong transaction state,
/// unknown user_id on geofence create). 409 is reserved for
/// `AlreadyExists`, which is what an idempotency-key collision returns.
pub fn status_to_http(code: Code) -> StatusCode {
    match code {
        Code::Ok => StatusCode::OK,
        Code::InvalidArgument | Code::OutOfRange => StatusCode::BAD_REQUEST,
        Code::Unauthenticated => StatusCode::UNAUTHORIZED,
        Code::PermissionDenied => StatusCode::FORBIDDEN,
        Code::NotFound => StatusCode::NOT_FOUND,
        Code::AlreadyExists | Code::Aborted => StatusCode::CONFLICT,
        Code::FailedPrecondition => StatusCode::UNPROCESSABLE_ENTITY,
        Code::ResourceExhausted => StatusCode::TOO_MANY_REQUESTS,
        Code::Cancelled => StatusCode::REQUEST_TIMEOUT,
        Code::DeadlineExceeded => StatusCode::GATEWAY_TIMEOUT,
        Code::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        Code::Unimplemented => StatusCode::NOT_IMPLEMENTED,
        Code::Internal | Code::Unknown | Code::DataLoss => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// Stable machine-readable string for the `error.code` field. SDKs branch
/// on this, so it must not drift with HTTP status changes.
fn code_slug(code: Code) -> &'static str {
    match code {
        Code::Ok => "ok",
        Code::InvalidArgument => "invalid_argument",
        Code::OutOfRange => "out_of_range",
        Code::Unauthenticated => "unauthenticated",
        Code::PermissionDenied => "permission_denied",
        Code::NotFound => "not_found",
        Code::AlreadyExists => "already_exists",
        Code::Aborted => "aborted",
        Code::FailedPrecondition => "failed_precondition",
        Code::ResourceExhausted => "resource_exhausted",
        Code::Cancelled => "cancelled",
        Code::DeadlineExceeded => "deadline_exceeded",
        Code::Unavailable => "unavailable",
        Code::Unimplemented => "unimplemented",
        Code::Internal | Code::Unknown | Code::DataLoss => "internal",
    }
}

const GENERIC_5XX: &str = "the request could not be completed";

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (http, slug, message) = match self {
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, "invalid_argument", msg),
            ApiError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, "unauthenticated", msg),
            ApiError::Upstream { service, status } => {
                let http = status_to_http(status.code());
                let slug = code_slug(status.code());
                // Log the upstream detail regardless; only 4xx echoes it back.
                if http.is_server_error() {
                    error!(
                        service,
                        code = ?status.code(),
                        detail = status.message(),
                        "upstream call failed"
                    );
                    (http, slug, GENERIC_5XX.to_string())
                } else {
                    warn!(
                        service,
                        code = ?status.code(),
                        detail = status.message(),
                        "upstream rejected request"
                    );
                    (http, slug, status.message().to_string())
                }
            }
        };

        metrics::counter!("atlas_gateway_errors_total", "code" => slug).increment(1);
        (
            http,
            Json(json!({ "error": { "code": slug, "message": message } })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_errors_map_to_4xx() {
        assert_eq!(
            status_to_http(Code::InvalidArgument),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            status_to_http(Code::Unauthenticated),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            status_to_http(Code::PermissionDenied),
            StatusCode::FORBIDDEN
        );
        assert_eq!(status_to_http(Code::NotFound), StatusCode::NOT_FOUND);
        assert_eq!(status_to_http(Code::AlreadyExists), StatusCode::CONFLICT);
        assert_eq!(
            status_to_http(Code::FailedPrecondition),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    #[test]
    fn server_errors_map_to_5xx() {
        assert_eq!(
            status_to_http(Code::Internal),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            status_to_http(Code::Unknown),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            status_to_http(Code::Unavailable),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            status_to_http(Code::DeadlineExceeded),
            StatusCode::GATEWAY_TIMEOUT
        );
    }

    /// An upstream 5xx must never echo the backend's message. This is the
    /// leak-prevention guarantee, so it gets a test rather than a comment.
    #[test]
    fn internal_upstream_message_is_not_leaked() {
        let err = ApiError::upstream(
            "payments",
            tonic::Status::internal("duplicate key value violates constraint payments_pkey"),
        );
        let body = format!("{:?}", err);
        assert!(
            body.contains("payments_pkey"),
            "precondition: detail present"
        );

        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn code_slugs_are_stable_for_sdk_branching() {
        assert_eq!(code_slug(Code::InvalidArgument), "invalid_argument");
        assert_eq!(code_slug(Code::AlreadyExists), "already_exists");
        // Every 5xx-ish code collapses to one slug so SDKs have a single
        // retry branch.
        assert_eq!(code_slug(Code::Internal), "internal");
        assert_eq!(code_slug(Code::Unknown), "internal");
        assert_eq!(code_slug(Code::DataLoss), "internal");
    }
}
