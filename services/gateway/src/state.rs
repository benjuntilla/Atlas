//! Shared application state: one gRPC client per backend service.
//!
//! Channels are built lazily (`connect_lazy`). That matters for boot
//! ordering — docker-compose and k8s both start the gateway before the
//! backends are necessarily accepting connections, and an eager connect
//! would crash-loop the gateway until every upstream was up. With lazy
//! channels the gateway binds its port immediately and individual
//! requests fail with `Unavailable` (503) until the backend arrives.
//!
//! `Channel` is cheap to clone (it is an `Arc` around a connection pool),
//! and tonic's generated clients need `&mut self`, so handlers clone the
//! client out of state rather than locking it.

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::sync::Arc;
use tonic::transport::{Channel, Endpoint};

use crate::config::Config;
use crate::pb::auth::auth_service_client::AuthServiceClient;
use crate::pb::geo::geo_engine_client::GeoEngineClient;
use crate::pb::payments::payments_service_client::PaymentsServiceClient;
use crate::tenant::ProjectCache;

#[derive(Clone)]
pub struct AppState {
    pub auth: AuthServiceClient<Channel>,
    pub geo: GeoEngineClient<Channel>,
    pub payments: PaymentsServiceClient<Channel>,
    /// Read-only pool for resolving project keys. Lazy for the same reason
    /// the gRPC channels are: the gateway must bind its port and answer
    /// health probes whether or not Postgres is up yet. A request that
    /// needs it while it is down fails with 503, which is the truth.
    pub pool: PgPool,
    pub projects: Arc<ProjectCache>,
}

impl AppState {
    pub fn connect(cfg: &Config) -> anyhow::Result<Self> {
        Ok(Self {
            auth: AuthServiceClient::new(endpoint(&cfg.auth_addr, cfg)?),
            geo: GeoEngineClient::new(endpoint(&cfg.geo_addr, cfg)?),
            payments: PaymentsServiceClient::new(endpoint(&cfg.payments_addr, cfg)?),
            pool: PgPoolOptions::new()
                .max_connections(cfg.database_pool_size)
                .acquire_timeout(std::time::Duration::from_secs(5))
                .connect_lazy(&cfg.database_url)?,
            projects: ProjectCache::new(cfg.project_cache_ttl),
        })
    }
}

fn endpoint(addr: &str, cfg: &Config) -> anyhow::Result<Channel> {
    Ok(Endpoint::from_shared(addr.to_string())?
        .timeout(cfg.upstream_timeout)
        .connect_timeout(cfg.upstream_connect_timeout)
        // Backends are long-lived gRPC servers; keepalive stops an idle
        // NAT or LB from silently dropping a pooled HTTP/2 connection and
        // turning the next request into a mystery timeout.
        .tcp_keepalive(Some(std::time::Duration::from_secs(60)))
        .connect_lazy())
}
