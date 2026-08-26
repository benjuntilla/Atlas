//! Library surface for `atlas-location-consumer`.
//!
//! # What this consumer is for
//!
//! Migration 0020 stamps every `geo.locations` row with
//! `expires_at = NOW() + 24 hours` and notes that the TTL is "enforced by
//! location-consumer". Until this crate existed, nothing enforced it:
//! rows accumulated forever, and because `nearby` filters on
//! `expires_at > NOW()` the table grew without bound while the useful
//! working set stayed small. Every proximity query paid for that.
//!
//! So the substantive job here is the [`reaper`] — a periodic batched
//! DELETE of expired rows.
//!
//! # Why it also consumes the topic
//!
//! The reaper alone would be a cron job. Subscribing to
//! `atlas.location.updates` earns its keep by making ingest observable:
//! throughput, decode failures, and the lag between a ping being recorded
//! on a device and being processed here. That last number is the one that
//! tells you whether the geospatial pipeline is keeping up, and nothing
//! else measures it.
//!
//! It deliberately does NOT write locations to Postgres — geo-engine
//! already does that synchronously on the gRPC path, and a second writer
//! would double-insert every ping.

pub mod config;
pub mod consumer;
pub mod metrics;
pub mod reaper;

pub mod pb {
    pub mod events {
        include!(concat!(env!("OUT_DIR"), "/atlas.events.rs"));
    }
}
