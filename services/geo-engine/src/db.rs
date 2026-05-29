//! Postgres connection pool setup.
//!
//! The pool is shared across the tonic service so every RPC handler reuses
//! the same connection budget. Sizing is configurable via DATABASE_POOL_SIZE
//! (default 20) since UpdateLocation traffic can be bursty.

use anyhow::Context;
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::time::Duration;

pub async fn connect(database_url: &str, pool_size: u32) -> anyhow::Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(pool_size)
        // Five seconds is long enough for a healthy postgres to respond and
        // short enough that a hung connection fails the RPC instead of
        // wedging the whole service.
        .acquire_timeout(Duration::from_secs(5))
        .connect(database_url)
        .await
        .with_context(|| format!("connecting to postgres at {database_url}"))
}
