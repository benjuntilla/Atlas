//! HTTP error type.
//!
//! The CLI surfaces failures as `"<operation> failed ({status}): {body}"`,
//! so the body text lands in front of a developer verbatim. It needs to be
//! readable prose, and it must never contain a database detail.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use tracing::error;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Unauthorized(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    /// Anything unexpected. The inner detail is logged, never returned.
    #[error("internal error")]
    Internal(#[from] anyhow::Error),
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        ApiError::Internal(anyhow::Error::new(e))
    }
}

impl ApiError {
    fn parts(&self) -> (StatusCode, &'static str) {
        match self {
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, "invalid_argument"),
            ApiError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, "unauthenticated"),
            ApiError::Forbidden(_) => (StatusCode::FORBIDDEN, "permission_denied"),
            ApiError::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
            ApiError::Conflict(_) => (StatusCode::CONFLICT, "already_exists"),
            ApiError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = self.parts();

        // A sqlx error can carry a constraint name, a column list, or the
        // connection string. None of that goes over the wire.
        let message = match &self {
            ApiError::Internal(e) => {
                error!(error = ?e, "control plane request failed");
                "the request could not be completed".to_string()
            }
            other => other.to_string(),
        };

        metrics::counter!("atlas_control_plane_errors_total", "code" => code).increment(1);
        (
            status,
            Json(json!({ "error": { "code": code, "message": message } })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_errors_keep_their_message() {
        let err = ApiError::NotFound("project 'ghost' not found".to_string());
        assert_eq!(err.parts().0, StatusCode::NOT_FOUND);
        assert_eq!(err.to_string(), "project 'ghost' not found");
    }

    /// A database error must not reach the caller — the CLI prints the
    /// response body straight to a terminal.
    #[test]
    fn database_errors_are_not_leaked() {
        let err: ApiError = sqlx::Error::RowNotFound.into();
        let (status, code) = err.parts();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(code, "internal");

        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
