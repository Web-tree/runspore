//! The reducer boundary. Mirrors `contracts/wit/machine/machine.wit` field for field.
//!
//! A reducer is pure: the same request always yields the same result, on every
//! host. It performs no I/O and reads no clock. Behavior is specified in
//! `spec/kernel.md`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub package_digest: String,
    pub kernel_digest: String,
    pub semantics_version: String,
    pub codec_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub event_id: String,
    pub sequence: u64,
    pub accepted_at_ms: u64,
    pub kind: String,
    /// Canonical JSON.
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub microsteps: u32,
    pub expression_operations: u32,
    pub max_state_bytes: u32,
    pub max_command_count: u32,
}

impl Default for Limits {
    /// The frozen budget every host passes under semantics 0.1.
    fn default() -> Self {
        Self {
            microsteps: 1_000,
            expression_operations: 10_000,
            max_state_bytes: 256 * 1024,
            max_command_count: 128,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionRequest {
    pub identity: Identity,
    /// Canonical workflow document.
    pub graph: Vec<u8>,
    /// `None` is legal only with a `run.started` event.
    pub snapshot: Option<Vec<u8>>,
    pub input_event: Envelope,
    pub frozen_limits: Limits,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Command {
    pub command_id: String,
    pub activation_id: String,
    pub kind: String,
    /// Canonical JSON.
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub node_id: Option<String>,
    /// Canonical JSON.
    pub details: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    /// Canonical state.
    pub snapshot: Vec<u8>,
    pub snapshot_digest: String,
    pub commands: Vec<Command>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FailureKind {
    InvalidInput,
    IncompatibleVersion,
    ResourceLimit,
    InvariantViolation,
}

/// A transition that produced no decision. Nothing may be committed for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{kind:?} {code}: {details}")]
pub struct Failure {
    pub kind: FailureKind,
    pub code: String,
    pub details: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Descriptor {
    pub abi_version: String,
    pub semantics_version: String,
    pub graph_format_version: String,
    pub codec_version: String,
}

/// One implementation of graph semantics behind any execution vehicle
/// (native call, Wasmtime component, transpiled component in a JS host).
pub trait Reducer: Send + Sync {
    fn describe(&self) -> Descriptor;

    /// Identifies the executing kernel artifact; recorded as `Identity.kernel_digest`.
    fn kernel_digest(&self) -> String;

    fn transition(&self, request: &TransitionRequest) -> Result<Decision, Failure>;
}
