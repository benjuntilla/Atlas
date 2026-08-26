//! Publishing side: `atlas.safety.alerts`.
//!
//! Behind a trait so the transition logic can be tested without a broker,
//! mirroring `RecordingFareEventPublisher` in payments-service.

use prost::Message;
use rdkafka::config::ClientConfig;
use rdkafka::producer::{FutureProducer, FutureRecord};
use std::time::Duration;

use crate::pb::events::SafetyAlertEvent;

/// How Atlas emits a safety alert.
///
/// `publish` must not return `Ok` until the broker has acked, because the
/// caller commits a database transaction on the strength of it.
pub trait AlertPublisher: Send + Sync {
    fn publish(
        &self,
        event: &SafetyAlertEvent,
    ) -> impl std::future::Future<Output = anyhow::Result<()>> + Send;
}

pub struct KafkaAlertPublisher {
    producer: FutureProducer,
    topic: String,
    ack_timeout: Duration,
}

impl KafkaAlertPublisher {
    pub fn new(brokers: &str, topic: String) -> anyhow::Result<Self> {
        let producer: FutureProducer = ClientConfig::new()
            .set("bootstrap.servers", brokers)
            // acks=all + idempotence: a safety alert is worth more than
            // the microseconds saved by fire-and-forget. This is the
            // opposite call from geo-engine's location producer, where a
            // dropped ping is replaced by the next one a second later.
            .set("acks", "all")
            .set("enable.idempotence", "true")
            .set("message.timeout.ms", "10000")
            .set("compression.type", "lz4")
            .create()?;
        Ok(Self {
            producer,
            topic,
            ack_timeout: Duration::from_secs(10),
        })
    }
}

impl AlertPublisher for KafkaAlertPublisher {
    async fn publish(&self, event: &SafetyAlertEvent) -> anyhow::Result<()> {
        let payload = event.encode_to_vec();
        // Key by user_id so one user's alerts stay ordered on one
        // partition — an ENTERED must never appear after the EXITED that
        // followed it.
        let record = FutureRecord::to(&self.topic)
            .payload(&payload)
            .key(&event.user_id);
        self.producer
            .send(record, self.ack_timeout)
            .await
            .map_err(|(e, _)| anyhow::anyhow!("publishing safety alert: {e}"))?;
        metrics::counter!("atlas_safety_alerts_published_total").increment(1);
        Ok(())
    }
}

/// In-memory publisher for tests. Records every alert so assertions can
/// check what the transition logic emitted.
#[derive(Default)]
pub struct RecordingAlertPublisher {
    published: std::sync::Mutex<Vec<SafetyAlertEvent>>,
    /// When set, every publish fails — used to prove that a publish
    /// failure rolls the membership change back.
    pub fail: bool,
}

impl RecordingAlertPublisher {
    pub fn failing() -> Self {
        Self {
            fail: true,
            ..Default::default()
        }
    }

    pub fn published(&self) -> Vec<SafetyAlertEvent> {
        self.published.lock().expect("not poisoned").clone()
    }
}

impl AlertPublisher for RecordingAlertPublisher {
    async fn publish(&self, event: &SafetyAlertEvent) -> anyhow::Result<()> {
        if self.fail {
            anyhow::bail!("publisher configured to fail");
        }
        self.published
            .lock()
            .expect("not poisoned")
            .push(event.clone());
        Ok(())
    }
}
