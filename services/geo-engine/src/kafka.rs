//! Kafka producer for `atlas.location.updates`.
//!
//! Design notes:
//!   * Payload is the protobuf `LocationUpdateEvent` from proto/events.proto
//!     (compiled into the generated `atlas.events` module).
//!   * Key is the user_id as UTF-8 bytes so all updates for a given user
//!     land on the same partition (ordering guarantee that downstream
//!     consumers rely on).
//!   * Producer enqueue is **fire-and-forget** on the gRPC hot path. The
//!     row is already in Postgres; Kafka is best-effort. A failed enqueue
//!     bumps a counter and logs at warn but never fails the RPC.

use prost::Message;
use rdkafka::config::ClientConfig;
use rdkafka::producer::{FutureProducer, FutureRecord};
use std::time::Duration;
use tracing::warn;

use crate::pb::events::LocationUpdateEvent;

#[derive(Clone)]
pub struct LocationProducer {
    producer: FutureProducer,
    topic: String,
}

impl LocationProducer {
    pub fn new(brokers: &str, topic: String) -> anyhow::Result<Self> {
        let producer: FutureProducer = ClientConfig::new()
            .set("bootstrap.servers", brokers)
            // 10s is well under the default 5min and prevents wedged
            // producer queues from holding service shutdown.
            .set("message.timeout.ms", "10000")
            // Compression cuts wire size for what is essentially small
            // numeric protobuf payloads — cheap to enable.
            .set("compression.type", "lz4")
            .create()?;
        Ok(Self { producer, topic })
    }

    /// Enqueue without awaiting delivery. Returns immediately.
    /// Errors are logged but never propagated — the RPC must not fail
    /// because Kafka is slow.
    pub fn enqueue(&self, event: &LocationUpdateEvent) {
        let payload = event.encode_to_vec();
        let key = event.user_id.clone();
        let topic = self.topic.clone();
        let producer = self.producer.clone();
        // Spawn a detached task so the producer queue blocking does not
        // hold the gRPC handler.
        tokio::spawn(async move {
            let record = FutureRecord::to(&topic).payload(&payload).key(&key);
            match producer.send(record, Duration::from_secs(0)).await {
                Ok(_) => {
                    metrics::counter!(
                        "atlas_kafka_messages_produced_total",
                        "topic" => topic.clone(),
                    )
                    .increment(1);
                }
                Err((e, _msg)) => {
                    metrics::counter!(
                        "atlas_kafka_produce_errors_total",
                        "topic" => topic.clone(),
                    )
                    .increment(1);
                    warn!(error = %e, topic = %topic, "kafka enqueue failed");
                }
            }
        });
    }
}
