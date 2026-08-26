//! Library surface for `atlas-gateway`, the public edge of the platform.
//!
//! # What this service is for
//!
//! Everything behind the gateway speaks gRPC on a private network and
//! trusts its callers. The gateway is the one process that does not: it
//! terminates untrusted HTTP, proves who the caller is, and only then
//! fans out to auth / geo / payments.
//!
//! # The trust boundary
//!
//! This is the important part. `services/geo-engine/src/service.rs`
//! documents that it trusts the `user_id` in each request body because
//! "the gateway validates JWTs and forwards already-authenticated
//! requests". That contract only holds if the gateway never forwards a
//! client-supplied identity. So it does not: every handler that needs a
//! `user_id` reads it from the validated token claims in [`extract::AuthUser`],
//! and the REST DTOs in [`routes`] deliberately have no `user_id` field for
//! a client to populate. A caller cannot ask about, move money from, or
//! write locations for anyone but themselves.
//!
//! # The tenant boundary
//!
//! Alongside the user identity there is a second, independent one: which
//! customer's application is asking. That comes from the `X-Atlas-Key`
//! header and is resolved in [`tenant`], which runs as a layer over every
//! `/v1` route so a newly added route cannot be tenant-unaware by
//! omission. As with `user_id`, no request DTO carries a project field —
//! a caller cannot name a project, only present a key that names one.
//!
//! Two RPCs are intentionally unreachable from here, per the notes in
//! proto/auth.proto and proto/payments.proto: `auth.IssueToken` (mints a
//! token for an arbitrary user_id) and `payments.DrainOutbox` (an ops
//! primitive). Neither has a route.

pub mod config;
pub mod error;
pub mod extract;
pub mod metrics;
pub mod ratelimit;
pub mod routes;
pub mod state;
pub mod tenant;
pub mod validate;

/// Generated protobuf clients, keyed by proto package name.
pub mod pb {
    pub mod auth {
        tonic::include_proto!("atlas.auth");
    }
    pub mod geo {
        tonic::include_proto!("atlas.geo");
    }
    pub mod payments {
        tonic::include_proto!("atlas.payments");
    }
}
