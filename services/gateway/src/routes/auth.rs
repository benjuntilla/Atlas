//! `atlas.auth` — the only namespace with unauthenticated routes.
//!
//! `POST /register` and `POST /login` are public by necessity. Everything
//! else takes an [`AuthUser`], which makes it authenticated.
//!
//! `auth.IssueToken` has no route here, deliberately: proto/auth.proto
//! marks it "Internal / refresh primitive. Not exposed via the gateway",
//! and for good reason — it mints a valid token for any `user_id` with no
//! credential check. Exposing it would be a complete authentication
//! bypass.

use axum::extract::State;
use axum::http::StatusCode;
use axum::{routing::get, routing::post, Json, Router};
use serde::{Deserialize, Serialize};
use tonic::Request;

use crate::error::ApiError;
use crate::extract::AuthUser;
use crate::pb::auth::{AuthRequest, RegisterRequest, RevokeTokenRequest};
use crate::state::AppState;
use crate::validate;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/register", post(register))
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/me", get(me))
}

#[derive(Debug, Deserialize)]
pub struct RegisterBody {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct RegisterOut {
    pub user_id: String,
}

async fn register(
    State(state): State<AppState>,
    Json(body): Json<RegisterBody>,
) -> Result<(StatusCode, Json<RegisterOut>), ApiError> {
    // Email/password rules (format, length, strength) live in the
    // auth-service domain layer. The gateway only rejects the empty case
    // so a blank form does not cost a gRPC round trip.
    if body.email.trim().is_empty() || body.password.is_empty() {
        return Err(ApiError::BadRequest(
            "email and password are required".to_string(),
        ));
    }

    let resp = state
        .auth
        .clone()
        .register(Request::new(RegisterRequest {
            email: body.email,
            password: body.password,
        }))
        .await
        .map_err(|s| ApiError::upstream("auth", s))?
        .into_inner();

    Ok((
        StatusCode::CREATED,
        Json(RegisterOut {
            user_id: resp.user_id,
        }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct LoginBody {
    pub email: String,
    pub password: String,
    /// Optional position stamped into the token's geospatial claims.
    pub lat: Option<f64>,
    pub lng: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct TokenOut {
    pub token: String,
    pub expires_at: i64,
}

async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginBody>,
) -> Result<Json<TokenOut>, ApiError> {
    if body.email.trim().is_empty() || body.password.is_empty() {
        return Err(ApiError::BadRequest(
            "email and password are required".to_string(),
        ));
    }

    // The proto has no optional scalars, and auth-service reads (0, 0) as
    // "no location supplied". Send zeros when the client omits both, but
    // validate whatever it did send — an out-of-range coordinate should
    // be a 400, not a silently stored bad claim.
    let (lat, lng) = match (body.lat, body.lng) {
        (Some(lat), Some(lng)) => {
            validate::lat_lng(lat, lng)?;
            (lat, lng)
        }
        (None, None) => (0.0, 0.0),
        _ => {
            return Err(ApiError::BadRequest(
                "lat and lng must be supplied together".to_string(),
            ))
        }
    };

    let resp = state
        .auth
        .clone()
        .authenticate(Request::new(AuthRequest {
            email: body.email,
            password: body.password,
            lat,
            lng,
        }))
        .await
        .map_err(|s| ApiError::upstream("auth", s))?
        .into_inner();

    Ok(Json(TokenOut {
        token: resp.token,
        expires_at: resp.expires_at,
    }))
}

#[derive(Debug, Serialize)]
pub struct LogoutOut {
    pub success: bool,
}

/// Revoke the caller's own token. The token comes from the validated
/// `Authorization` header, so a caller can only ever log itself out —
/// there is no body field naming a token to revoke.
async fn logout(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<LogoutOut>, ApiError> {
    let resp = state
        .auth
        .clone()
        .revoke_token(Request::new(RevokeTokenRequest { token: user.token }))
        .await
        .map_err(|s| ApiError::upstream("auth", s))?
        .into_inner();

    Ok(Json(LogoutOut {
        success: resp.success,
    }))
}

#[derive(Debug, Serialize)]
pub struct MeOut {
    pub user_id: String,
    pub session_id: String,
    pub last_lat: f64,
    pub last_lng: f64,
    pub issued_at: i64,
    pub expires_at: i64,
}

/// Echo the validated claims. Useful for SDK session bootstrapping and
/// for confirming a token is still live without a mutating call.
async fn me(user: AuthUser) -> Json<MeOut> {
    Json(MeOut {
        user_id: user.user_id,
        session_id: user.session_id,
        last_lat: user.last_lat,
        last_lng: user.last_lng,
        issued_at: user.issued_at,
        expires_at: user.expires_at,
    })
}
