//! Environment-driven configuration.
//!
//! Defaults point at the docker-compose topology so `cargo run -p
//! atlas-gateway` works against a local stack with no env vars set.
//! Production overrides everything through the container environment.

use std::env;
use std::net::SocketAddr;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Config {
    /// Public REST listener. This is the only port exposed to the internet.
    pub http_addr: SocketAddr,
    /// Prometheus scrape listener. Kept on a separate port so `/metrics`
    /// is never reachable from the public interface.
    pub metrics_addr: SocketAddr,
    pub auth_addr: String,
    pub geo_addr: String,
    pub payments_addr: String,
    /// Per-RPC deadline applied to every upstream call. Without this a
    /// wedged backend would hold gateway connections until the client
    /// gives up.
    pub upstream_timeout: Duration,
    /// TCP connect timeout for a lazily-established upstream channel.
    pub upstream_connect_timeout: Duration,

    /// Per-replica request limiting. See `ratelimit.rs` for what this does
    /// and does not protect.
    pub rate_limit: crate::ratelimit::RateLimitConfig,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            http_addr: parse_addr("HTTP_ADDR", "0.0.0.0:8080"),
            metrics_addr: parse_addr("METRICS_ADDR", "0.0.0.0:9090"),
            auth_addr: env::var("AUTH_SERVICE_ADDR")
                .unwrap_or_else(|_| "http://localhost:50051".to_string()),
            geo_addr: env::var("GEO_ENGINE_ADDR")
                .unwrap_or_else(|_| "http://localhost:50052".to_string()),
            payments_addr: env::var("PAYMENTS_SERVICE_ADDR")
                .unwrap_or_else(|_| "http://localhost:50053".to_string()),
            upstream_timeout: parse_secs("UPSTREAM_TIMEOUT_SECONDS", 10),
            upstream_connect_timeout: parse_secs("UPSTREAM_CONNECT_TIMEOUT_SECONDS", 5),
            rate_limit: crate::ratelimit::RateLimitConfig {
                default_per_minute: parse_u32("RATE_LIMIT_PER_MINUTE", 600),
                auth_per_minute: parse_u32("RATE_LIMIT_AUTH_PER_MINUTE", 10),
                // Defaults to 0: trust nothing in X-Forwarded-For unless the
                // operator states how many proxies actually sit in front.
                // Behind the ingress-nginx in infra/k8s this is 1.
                trusted_proxy_hops: parse_u32("TRUSTED_PROXY_HOPS", 0) as usize,
                enabled: env::var("RATE_LIMIT_ENABLED")
                    .map(|v| v != "false" && v != "0")
                    .unwrap_or(true),
            },
        }
    }
}

fn parse_addr(var: &str, default: &str) -> SocketAddr {
    env::var(var)
        .unwrap_or_else(|_| default.to_string())
        .parse()
        .unwrap_or_else(|_| panic!("{var} must be a valid socket address"))
}

fn parse_u32(var: &str, default: u32) -> u32 {
    env::var(var)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn parse_secs(var: &str, default: u64) -> Duration {
    Duration::from_secs(
        env::var(var)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default),
    )
}
