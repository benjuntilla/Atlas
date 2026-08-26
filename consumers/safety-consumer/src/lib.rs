//! Library surface for `atlas-safety-consumer`.
//!
//! # What this closes
//!
//! `SafetyAlertEvent` has existed in proto/events.proto since Phase 1 and
//! `atlas.safety.alerts` has had a Strimzi topic manifest for just as
//! long, but nothing in the platform ever produced one. Geofencing was
//! only reachable as a synchronous question — `TriggerGeofenceCheck` told
//! a caller which fences it was inside *if it thought to ask*. Nothing
//! noticed a user crossing a boundary on its own.
//!
//! This consumer is the missing half. It reads every location ping off
//! `atlas.location.updates`, works out which of that user's geofences
//! they are now inside, and compares that against what they were inside
//! before. The difference is the alert:
//!
//! ```text
//!   in fences now  - in fences before  =  GEOFENCE_ENTERED
//!   in fences before - in fences now   =  GEOFENCE_EXITED
//! ```
//!
//! # Why membership lives in Postgres
//!
//! A geofence alert is about a transition, so the previous state has to
//! come from somewhere. `geo.geofence_memberships` (migration 0023) holds
//! it rather than an in-process HashMap, which buys three things: it
//! survives restarts (no alert storm on every deploy), it lets instances
//! share one view of the world so the consumer scales horizontally, and
//! it makes reprocessing free of side effects — replaying a ping that has
//! already been applied produces an empty diff and emits nothing. That
//! last property is what makes at-least-once Kafka delivery safe here.
//!
//! # Delivery guarantees
//!
//! Offsets are committed only after the diff is persisted and its alerts
//! are acked by the broker, so a crash mid-batch replays rather than
//! drops. Replay is safe for the reason above. Events are keyed by
//! `user_id` on the producing side, so one user's pings always land on
//! one partition and are processed in order.

pub mod alerts;
pub mod config;
pub mod consumer;
pub mod metrics;
pub mod producer;

pub mod pb {
    pub mod events {
        include!(concat!(env!("OUT_DIR"), "/atlas.events.rs"));
    }
}
