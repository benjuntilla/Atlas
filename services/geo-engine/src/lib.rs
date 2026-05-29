//! Library surface for `atlas-geo-engine`. The binary in `src/main.rs`
//! consumes this crate; integration tests in `tests/` also do.
//!
//! Everything that is not `pub` here is private to the crate. The
//! integration tests in `tests/queries_test.rs` exercise the `queries`
//! module against a real Postgres.

pub mod config;
pub mod db;
pub mod health;
pub mod kafka;
pub mod metrics;
pub mod queries;
pub mod service;

/// Generated protobuf code lives here. The `tonic::include_proto!` macro
/// pulls in the file emitted by `build.rs` keyed by proto package name.
pub mod pb {
    pub mod geo {
        tonic::include_proto!("atlas.geo");
    }
    pub mod events {
        tonic::include_proto!("atlas.events");
    }
}
