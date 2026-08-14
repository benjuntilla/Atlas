//! Shared application state.

use sqlx::PgPool;
use std::sync::Arc;

use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub cfg: Arc<Config>,
    /// Reused for the gateway metrics scrape. A fresh `reqwest::Client`
    /// per request would discard the connection pool and the TLS session
    /// cache.
    pub http: reqwest::Client,
}

impl AppState {
    pub fn new(pool: PgPool, cfg: Config) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(cfg.probe_timeout)
            .build()?;
        Ok(Self {
            pool,
            cfg: Arc::new(cfg),
            http,
        })
    }
}
