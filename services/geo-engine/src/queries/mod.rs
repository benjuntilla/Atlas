//! SQL query modules.
//!
//! One file per logical area (locations, routes, geofences). Each function
//! takes a `&PgPool` (or `&mut PgConnection`) plus typed args and returns
//! a typed result. The service layer translates these to gRPC responses.

pub mod geofences;
pub mod locations;
pub mod routes;
