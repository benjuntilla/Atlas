//! Postgres pool.
//!
//! The control plane is low-traffic compared to the data-plane services —
//! it sees CLI invocations, not user requests — so the default pool is
//! deliberately smaller than geo-engine's 20.

use anyhow::Context;
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::time::Duration;

pub async fn connect(database_url: &str, pool_size: u32) -> anyhow::Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(pool_size)
        .acquire_timeout(Duration::from_secs(5))
        .connect(database_url)
        .await
        .with_context(|| format!("connecting to postgres at {database_url}"))
}
