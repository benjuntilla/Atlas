//! API key format, re-exported from the shared `atlas-keys` crate.
//!
//! The definitions moved out of this file so the gateway can resolve keys
//! against the same format the control plane mints them in. The module
//! path stays because it is what every call site in this service uses, and
//! because "where keys come from" is genuinely control-plane vocabulary.

pub use atlas_keys::*;
