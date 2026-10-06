//! The frozen contract every Runspore crate builds against.
//!
//! - [`canonical`]: the one JSON encoding used for anything hashed or stored.
//! - [`digest`]: digests and derived identities.
//! - [`model`]: workflow document, run state, event and command payloads.
//! - [`reducer`]: the pure kernel boundary (mirrors the WIT).
//! - [`store`]: the semantic store protocol.
//! - [`failpoint`]: crash injection for durability tests (native targets only).
//!
//! Changing a wire shape or an identity derivation here changes
//! [`model::SEMANTICS_VERSION`]. Behavior lives in `spec/`.

pub mod canonical;
pub mod digest;
#[cfg(not(target_family = "wasm"))]
pub mod failpoint;
pub mod model;
pub mod reducer;
pub mod store;

pub use serde_json::Value;
