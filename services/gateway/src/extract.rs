//! The `AuthUser` extractor — the gateway's entire authentication story.
//!
//! Any handler that takes an [`AuthUser`] argument is authenticated: axum
//! runs the extractor before the handler body, and a failure short-circuits
//! into a 401 without the handler ever running. Handlers that omit it are
//! public by construction (register, login, health).
//!
//! Identity comes from `auth.ValidateToken`, never from the request body.
//! The auth-service keeps a 30s in-process cache keyed by token, so this
//! is one cheap gRPC hop rather than a Postgres hit per request — see the
//! `TokenValidationCache` note in `AuthGrpcService`.
//!
//! There is deliberately no gateway-side token cache. A second cache layer
//! would extend the revocation window past the 30s the auth-service
//! already accepts, and `RevokeToken` fans out over Kafka to auth-service
//! instances only — the gateway would never hear about it.

use axum::async_trait;
use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use tonic::Request;

use crate::error::ApiError;
use crate::pb::auth::ValidateTokenRequest;
use crate::state::AppState;
use crate::validate;

/// A caller whose bearer token has been validated upstream this request.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: String,
    pub session_id: String,
    /// Last known position stamped into the token at login. Present only
    /// if the client sent coordinates to `POST /v1/auth/login`; (0, 0) if
    /// it did not, matching the proto's lack of optionality.
    pub last_lat: f64,
    pub last_lng: f64,
    pub issued_at: i64,
    pub expires_at: i64,
    /// The raw token, kept so `POST /v1/auth/logout` can pass it back to
    /// `RevokeToken` without the handler re-reading the header.
    pub token: String,
}

#[async_trait]
impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(AUTHORIZATION)
            .ok_or_else(|| ApiError::Unauthorized("missing Authorization header".to_string()))?
            .to_str()
            .map_err(|_| {
                ApiError::Unauthorized("Authorization header is not valid UTF-8".to_string())
            })?;

        let token = validate::bearer_token(header)?.to_string();

        let claims = state
            .auth
            .clone()
            .validate_token(Request::new(ValidateTokenRequest {
                token: token.clone(),
            }))
            .await
            .map_err(|s| ApiError::upstream("auth", s))?
            .into_inner();

        Ok(AuthUser {
            user_id: claims.user_id,
            session_id: claims.session_id,
            last_lat: claims.last_lat,
            last_lng: claims.last_lng,
            issued_at: claims.issued_at,
            expires_at: claims.expires_at,
            token,
        })
    }
}
