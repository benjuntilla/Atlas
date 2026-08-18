//! `atlas.payments` — deposits, wallet reads, and transaction initiation.
//!
//! Deposits are the only way money enters the platform. The route is safe
//! to expose because the wallet credited is the token's subject: a caller
//! can only top up their own balance.
//!
//! # Why settle and refund are not routed here
//!
//! `payments.SettleTransaction` and `payments.RefundTransaction` take a
//! bare `transaction_id` and nothing else. There is no `GetTransaction`
//! RPC, so the gateway has no way to ask "does this transaction belong to
//! the caller?" before forwarding. Routing them as-is would let any
//! authenticated user settle or refund any transaction in the system by
//! id — moving other people's money — and no amount of gateway-side
//! validation can prevent that without an ownership signal from the
//! payments service.
//!
//! The trust boundary is only meaningful if the gateway declines to
//! forward calls it cannot authorize, so these two stay internal, the
//! same way proto/payments.proto already marks `DrainOutbox` internal.
//!
//! This is not a gap in the money flow. Settlement is meant to be driven
//! by ride lifecycle events: `initiate` enqueues a `RIDE_ACCEPTED` fare
//! event to the outbox, and the Phase 6 fare-consumer reacts to
//! `RIDE_COMPLETED` / `RIDE_CANCELLED` by settling or refunding
//! server-side. A client-initiated "settle my own fare" endpoint is not
//! part of that design.
//!
//! If a client-facing settle is wanted later, the prerequisite is an
//! ownership check in the payments contract — a `user_id` on
//! `SettleRequest`/`RefundRequest` verified against the transaction's
//! wallets — not a route added here.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::{routing::get, routing::post, Json, Router};
use serde::{Deserialize, Serialize};
use tonic::Request;

use crate::error::ApiError;
use crate::extract::AuthUser;
use crate::pb::payments::{DepositRequest, TransactionRequest, WalletRequest};
use crate::state::AppState;
use crate::validate;

/// Standard header name for idempotency, as used by Stripe et al. SDK
/// users reach for this before they read our field docs, so accept it.
const IDEMPOTENCY_HEADER: &str = "idempotency-key";

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/wallet", get(wallet))
        .route("/deposits", post(deposit))
        .route("/transactions", post(initiate))
}

#[derive(Debug, Serialize)]
pub struct WalletOut {
    pub balance_cents: i64,
    pub currency: String,
}

/// Read the caller's own wallet. There is no `GET /wallet/:user_id` —
/// the id comes from the token, so one user cannot read another's
/// balance.
async fn wallet(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<WalletOut>, ApiError> {
    let resp = state
        .payments
        .clone()
        .get_wallet_balance(Request::new(WalletRequest {
            project_id: user.project_id,
            user_id: user.user_id,
        }))
        .await
        .map_err(|s| ApiError::upstream("payments", s))?
        .into_inner();

    Ok(Json(WalletOut {
        balance_cents: resp.balance_cents,
        currency: resp.currency,
    }))
}

#[derive(Debug, Deserialize)]
pub struct DepositBody {
    /// Must be > 0.
    pub amount_cents: i64,
    /// Optional here for the same reason as on transactions: the
    /// `Idempotency-Key` header is the conventional place. One is required.
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DepositOut {
    pub transaction_id: String,
    pub status: String,
    /// Balance after the deposit, so the caller need not re-read the wallet.
    pub balance_cents: i64,
}

/// Add funds to the caller's own wallet.
///
/// Unlike settle and refund, this one is safe to route: the wallet being
/// credited is the token's subject, so there is no ownership question for
/// the gateway to answer. A caller can only ever top up themselves.
async fn deposit(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    Json(body): Json<DepositBody>,
) -> Result<(StatusCode, Json<DepositOut>), ApiError> {
    let amount = validate::amount_cents(body.amount_cents)?;

    let key_from_header = headers
        .get(IDEMPOTENCY_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let raw_key = key_from_header.or(body.idempotency_key).unwrap_or_default();
    let idempotency_key = validate::idempotency_key(&raw_key)?.to_string();

    let resp = state
        .payments
        .clone()
        .deposit(Request::new(DepositRequest {
            project_id: user.project_id,
            user_id: user.user_id,
            amount_cents: amount,
            idempotency_key,
        }))
        .await
        .map_err(|s| ApiError::upstream("payments", s))?
        .into_inner();

    Ok((
        StatusCode::CREATED,
        Json(DepositOut {
            transaction_id: resp.transaction_id,
            status: resp.status,
            balance_cents: resp.balance_cents,
        }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct InitiateBody {
    pub to_user_id: String,
    pub amount_cents: i64,
    /// Optional in the body because the `Idempotency-Key` header is the
    /// more conventional place to put it. One of the two is required.
    pub idempotency_key: Option<String>,
    pub ride_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TransactionOut {
    pub transaction_id: String,
    pub status: String,
}

/// Start a transaction from the caller's wallet.
///
/// `from_user_id` is the token's subject and is not accepted from the
/// body, so a caller can only ever move their own money.
async fn initiate(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    Json(body): Json<InitiateBody>,
) -> Result<(StatusCode, Json<TransactionOut>), ApiError> {
    let amount = validate::amount_cents(body.amount_cents)?;

    if body.to_user_id.trim().is_empty() {
        return Err(ApiError::BadRequest("to_user_id is required".to_string()));
    }
    // Caught upstream too, but the upstream message says "from_user_id",
    // which names a field this API does not have — confusing for anyone
    // reading it against the REST docs.
    if body.to_user_id.trim() == user.user_id {
        return Err(ApiError::BadRequest(
            "to_user_id must differ from the authenticated user".to_string(),
        ));
    }

    // Header wins over body: it is the more specific signal, and a client
    // sending both almost certainly means the one its HTTP layer set.
    let key_from_header = headers
        .get(IDEMPOTENCY_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let raw_key = key_from_header.or(body.idempotency_key).unwrap_or_default();
    let idempotency_key = validate::idempotency_key(&raw_key)?.to_string();

    let resp = state
        .payments
        .clone()
        .initiate_transaction(Request::new(TransactionRequest {
            project_id: user.project_id,
            from_user_id: user.user_id,
            to_user_id: body.to_user_id.trim().to_string(),
            amount_cents: amount,
            idempotency_key,
            // Blank is meaningful: payments reads an empty ride_id as
            // NULL rather than trying to parse it as a UUID.
            ride_id: body.ride_id.unwrap_or_default(),
        }))
        .await
        .map_err(|s| ApiError::upstream("payments", s))?
        .into_inner();

    Ok((
        StatusCode::CREATED,
        Json(TransactionOut {
            transaction_id: resp.transaction_id,
            status: resp.status,
        }),
    ))
}
