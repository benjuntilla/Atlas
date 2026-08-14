//! Kafka ingest loop for `atlas.location.updates`.
//!
//! Unlike location-consumer, processing here has side effects — database
//! writes and published alerts — so auto-commit is off. An offset is
//! committed only after `apply_position` has published and committed,
//! which makes a crash replay rather than skip.

use prost::Message;
use rdkafka::config::ClientConfig;
use rdkafka::consumer::{CommitMode, Consumer, StreamConsumer};
use rdkafka::message::Message as _;
use sqlx::PgPool;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::alerts;
use crate::pb::events::LocationUpdateEvent;
use crate::producer::AlertPublisher;

pub fn build(brokers: &str, group: &str) -> anyhow::Result<StreamConsumer> {
    let consumer: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", brokers)
        .set("group.id", group)
        .set("auto.offset.reset", "earliest")
        // Manual commit: see the module note.
        .set("enable.auto.commit", "false")
        .set("session.timeout.ms", "10000")
        .create()?;
    Ok(consumer)
}

/// Outcome of handling one message, so the caller knows whether the
/// offset may advance.
#[derive(Debug, PartialEq, Eq)]
pub enum Handled {
    /// Processed, or skipped for a reason that replaying will not fix.
    Commit,
    /// A transient failure. Leave the offset where it is so the message
    /// is redelivered.
    Retry,
}

pub async fn run(
    consumer: StreamConsumer,
    pool: PgPool,
    publisher: impl AlertPublisher,
    topic: &str,
    shutdown: impl std::future::Future<Output = ()>,
) {
    if let Err(e) = consumer.subscribe(&[topic]) {
        warn!(error = %e, topic, "subscribe failed");
        return;
    }
    info!(topic, "watching for geofence crossings");

    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = &mut shutdown => {
                info!("shutdown signal received, stopping consumer");
                return;
            }
            result = consumer.recv() => {
                let msg = match result {
                    Ok(m) => m,
                    Err(e) => {
                        metrics::counter!("atlas_safety_consumer_errors_total", "kind" => "recv")
                            .increment(1);
                        warn!(error = %e, "kafka recv failed");
                        continue;
                    }
                };

                match handle_payload(&pool, &publisher, msg.payload()).await {
                    Handled::Commit => {
                        if let Err(e) = consumer.commit_message(&msg, CommitMode::Async) {
                            warn!(error = %e, "offset commit failed");
                        }
                    }
                    Handled::Retry => {
                        // Deliberately no commit. The broker redelivers
                        // after the session times out or on the next
                        // rebalance, and `apply_position` is safe to
                        // repeat.
                        metrics::counter!(
                            "atlas_safety_consumer_errors_total", "kind" => "retryable"
                        )
                        .increment(1);
                    }
                }
            }
        }
    }
}

async fn handle_payload(
    pool: &PgPool,
    publisher: &impl AlertPublisher,
    payload: Option<&[u8]>,
) -> Handled {
    let Some(bytes) = payload else {
        metrics::counter!("atlas_safety_consumer_errors_total", "kind" => "empty_payload")
            .increment(1);
        return Handled::Commit;
    };

    let event = match LocationUpdateEvent::decode(bytes) {
        Ok(e) => e,
        Err(e) => {
            // Undecodable means a producer/consumer schema mismatch.
            // Replaying will not fix it, and refusing to commit would
            // wedge the partition behind one bad message forever.
            metrics::counter!("atlas_safety_consumer_errors_total", "kind" => "decode")
                .increment(1);
            warn!(error = %e, "failed to decode LocationUpdateEvent; skipping");
            return Handled::Commit;
        }
    };

    let user_id = match Uuid::parse_str(&event.user_id) {
        Ok(id) => id,
        Err(_) => {
            // Same reasoning: a non-UUID user_id is malformed data, not a
            // transient fault.
            metrics::counter!("atlas_safety_consumer_errors_total", "kind" => "bad_user_id")
                .increment(1);
            warn!(user_id = %event.user_id, "location event has a non-UUID user_id; skipping");
            return Handled::Commit;
        }
    };

    // The proto has no optional scalars, so an unset timestamp arrives as
    // 0. Stamp the alert with now rather than the epoch.
    let occurred_at = if event.recorded_at > 0 {
        event.recorded_at
    } else {
        chrono::Utc::now().timestamp()
    };

    match alerts::apply_position(pool, publisher, user_id, event.lat, event.lng, occurred_at).await
    {
        Ok(t) => {
            if !t.is_empty() {
                info!(
                    %user_id,
                    entered = t.entered.len(),
                    exited = t.exited.len(),
                    "geofence transition"
                );
            } else {
                debug!(%user_id, "no geofence transition");
            }
            Handled::Commit
        }
        Err(e) => {
            // Postgres or Kafka is unhappy. Both are transient by nature,
            // so hold the offset and let it redeliver.
            warn!(error = %e, %user_id, "failed to apply position; will retry");
            Handled::Retry
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::producer::RecordingAlertPublisher;

    /// A pool pointed at a port nothing listens on.
    ///
    /// Most of these tests return before touching it; the one that does
    /// touch it expects the connection to fail. The short acquire timeout
    /// is what keeps that test sub-second instead of sitting on sqlx's
    /// 30s default.
    fn unconnected_pool() -> PgPool {
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(std::time::Duration::from_millis(250))
            .connect_lazy("postgres://atlas:atlas_dev@127.0.0.1:59999/atlas")
            .expect("lazy pool")
    }

    #[tokio::test]
    async fn empty_payload_commits_rather_than_wedging() {
        let publisher = RecordingAlertPublisher::default();
        let handled = handle_payload(&unconnected_pool(), &publisher, None).await;
        assert_eq!(handled, Handled::Commit);
        assert!(publisher.published().is_empty());
    }

    #[tokio::test]
    async fn undecodable_payload_commits_rather_than_wedging() {
        let publisher = RecordingAlertPublisher::default();
        let garbage = [0xffu8, 0xff, 0xff, 0xff];
        let handled = handle_payload(&unconnected_pool(), &publisher, Some(&garbage)).await;
        assert_eq!(
            handled,
            Handled::Commit,
            "a poison message must not block the partition"
        );
    }

    #[tokio::test]
    async fn non_uuid_user_id_commits_rather_than_wedging() {
        let publisher = RecordingAlertPublisher::default();
        let event = LocationUpdateEvent {
            user_id: "not-a-uuid".to_string(),
            lat: 1.0,
            lng: 2.0,
            recorded_at: 1_700_000_000,
        };
        let handled = handle_payload(
            &unconnected_pool(),
            &publisher,
            Some(&event.encode_to_vec()),
        )
        .await;
        assert_eq!(handled, Handled::Commit);
        assert!(publisher.published().is_empty());
    }

    /// A well-formed event against an unreachable database must NOT
    /// commit — that is the case where replay is the correct response.
    #[tokio::test]
    async fn database_failure_holds_the_offset() {
        let publisher = RecordingAlertPublisher::default();
        let event = LocationUpdateEvent {
            user_id: Uuid::from_bytes([1; 16]).to_string(),
            lat: 33.4484,
            lng: -112.0740,
            recorded_at: 1_700_000_000,
        };
        let handled = handle_payload(
            &unconnected_pool(),
            &publisher,
            Some(&event.encode_to_vec()),
        )
        .await;
        assert_eq!(handled, Handled::Retry);
    }
}
