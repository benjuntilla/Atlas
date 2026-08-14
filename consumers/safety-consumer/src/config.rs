//! Environment-driven configuration, defaulting to the docker-compose
//! topology.

use std::env;
use std::net::SocketAddr;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub database_pool_size: u32,
    pub kafka_brokers: String,
    /// Input: the same topic geo-engine produces to.
    pub topic_location_updates: String,
    /// Output: the topic nothing produced to before this consumer.
    pub topic_safety_alerts: String,
    /// Shared group — each ping must be handled once, not broadcast.
    pub consumer_group: String,
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
                .unwrap_or(10),
            kafka_brokers: env::var("KAFKA_BROKERS")
                .unwrap_or_else(|_| "localhost:9092".to_string()),
            topic_location_updates: env::var("KAFKA_TOPIC_LOCATION_UPDATES")
                .unwrap_or_else(|_| "atlas.location.updates".to_string()),
            topic_safety_alerts: env::var("KAFKA_TOPIC_SAFETY_ALERTS")
                .unwrap_or_else(|_| "atlas.safety.alerts".to_string()),
            consumer_group: env::var("CONSUMER_GROUP")
                .unwrap_or_else(|_| "atlas-safety-consumer".to_string()),
            metrics_addr: env::var("METRICS_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:9090".to_string())
                .parse()
                .expect("METRICS_ADDR must be a valid socket address"),
        }
    }
}
