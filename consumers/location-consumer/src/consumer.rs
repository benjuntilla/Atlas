//! Kafka ingest loop for `atlas.location.updates`.
//!
//! This loop is observability-only: it decodes each event and records
//! metrics. It writes nothing, because geo-engine has already persisted
//! the row synchronously by the time the event reaches Kafka.
//!
//! Auto-commit is on. Losing or replaying an offset here costs a metric
//! sample, not data — which is exactly the case where auto-commit is the
//! right trade. The safety-consumer, whose processing has side effects,
//! commits manually instead.

use prost::Message;
use rdkafka::config::ClientConfig;
use rdkafka::consumer::{Consumer, StreamConsumer};
use rdkafka::message::Message as _;
use tracing::{info, warn};

use crate::pb::events::LocationUpdateEvent;

pub fn build(brokers: &str, group: &str) -> anyhow::Result<StreamConsumer> {
    let consumer: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", brokers)
        .set("group.id", group)
        // Start from the beginning on a brand-new group so a first deploy
        // reports on the existing backlog rather than silently skipping it.
        .set("auto.offset.reset", "earliest")
        .set("enable.auto.commit", "true")
        .set("session.timeout.ms", "10000")
        .create()?;
    Ok(consumer)
}

/// Compute the delay between a ping being recorded on a device and being
/// processed here, in seconds.
///
/// Returns `None` when the client sent no timestamp (the proto has no
/// optional scalars, so "unset" arrives as 0) or when the timestamp is in
/// the future — a device with a wrong clock would otherwise contribute a
/// negative sample and quietly drag the average down.
pub fn ingest_lag_seconds(recorded_at: i64, now: i64) -> Option<f64> {
    if recorded_at <= 0 || recorded_at > now {
        return None;
    }
    Some((now - recorded_at) as f64)
}

/// Consume until the shutdown future resolves.
pub async fn run(
    consumer: StreamConsumer,
    topic: &str,
    shutdown: impl std::future::Future<Output = ()>,
) {
    if let Err(e) = consumer.subscribe(&[topic]) {
        warn!(error = %e, topic, "subscribe failed");
        return;
    }
    info!(topic, "consuming location updates");

    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = &mut shutdown => {
                info!("shutdown signal received, stopping ingest loop");
                return;
            }
            result = consumer.recv() => {
                match result {
                    Err(e) => {
                        metrics::counter!("atlas_location_consumer_errors_total", "kind" => "recv")
                            .increment(1);
                        warn!(error = %e, "kafka recv failed");
                    }
                    Ok(msg) => handle_message(msg.payload()),
                }
            }
        }
    }
}

fn handle_message(payload: Option<&[u8]>) {
    let Some(bytes) = payload else {
        // A null payload is a tombstone. This topic is not compacted, so
        // it should not happen; count it rather than panicking.
        metrics::counter!("atlas_location_consumer_errors_total", "kind" => "empty_payload")
            .increment(1);
        return;
    };

    match LocationUpdateEvent::decode(bytes) {
        Ok(event) => {
            metrics::counter!("atlas_location_events_consumed_total").increment(1);
            if let Some(lag) = ingest_lag_seconds(event.recorded_at, chrono::Utc::now().timestamp())
            {
                metrics::histogram!("atlas_location_ingest_lag_seconds").record(lag);
            }
        }
        Err(e) => {
            // A payload we cannot decode is a schema mismatch between a
            // producer and this consumer. Skip it: blocking the partition
            // on one bad message would stall every well-formed one behind
            // it, and the counter makes the problem visible.
            metrics::counter!("atlas_location_consumer_errors_total", "kind" => "decode")
                .increment(1);
            warn!(error = %e, "failed to decode LocationUpdateEvent; skipping");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000;

    #[test]
    fn lag_is_the_difference_in_seconds() {
        assert_eq!(ingest_lag_seconds(NOW - 30, NOW), Some(30.0));
        assert_eq!(ingest_lag_seconds(NOW, NOW), Some(0.0));
    }

    #[test]
    fn unset_timestamps_produce_no_sample() {
        assert_eq!(ingest_lag_seconds(0, NOW), None);
        assert_eq!(ingest_lag_seconds(-1, NOW), None);
    }

    /// A device with a fast clock must not contribute negative lag.
    #[test]
    fn future_timestamps_produce_no_sample() {
        assert_eq!(ingest_lag_seconds(NOW + 60, NOW), None);
    }

    #[test]
    fn empty_payload_is_counted_not_panicked() {
        handle_message(None);
        handle_message(Some(&[0xff, 0xff, 0xff]));
    }

    #[test]
    fn well_formed_event_decodes() {
        let event = LocationUpdateEvent {
            user_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            lat: 33.4484,
            lng: -112.0740,
            recorded_at: NOW,
        };
        let encoded = event.encode_to_vec();
        let decoded = LocationUpdateEvent::decode(encoded.as_slice()).unwrap();
        assert_eq!(decoded.user_id, event.user_id);
        assert_eq!(decoded.recorded_at, NOW);
    }
}
