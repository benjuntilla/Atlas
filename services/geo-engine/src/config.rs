//! Environment-driven configuration.
//!
//! Everything has a dev-friendly default so `cargo run` against a local
//! docker-compose works without exporting a single env var. Production
//! deploys override via the container environment.

use std::env;
use std::net::SocketAddr;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub database_pool_size: u32,
    pub kafka_brokers: String,
    pub kafka_topic_location_updates: String,
    pub grpc_addr: SocketAddr,
    pub metrics_addr: SocketAddr,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://atlas:atlas_dev@localhost:5432/atlas".to_string()),
            database_pool_size: env::var("DATABASE_POOL_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(20),
            kafka_brokers: env::var("KAFKA_BROKERS")
                .unwrap_or_else(|_| "localhost:9092".to_string()),
            kafka_topic_location_updates: env::var("KAFKA_TOPIC_LOCATION_UPDATES")
                .unwrap_or_else(|_| "atlas.location.updates".to_string()),
            grpc_addr: env::var("GRPC_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:50052".to_string())
                .parse()
                .expect("GRPC_ADDR must be a valid socket address"),
            metrics_addr: env::var("METRICS_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:9090".to_string())
                .parse()
                .expect("METRICS_ADDR must be a valid socket address"),
        }
    }
}
