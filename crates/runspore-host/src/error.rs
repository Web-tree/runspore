use std::fmt;

use runspore_schema::{InputMisfit, Misfit};
use runspore_types::model::code;
use runspore_types::reducer::Failure;
use runspore_types::store::StoreFailure;
use serde_json::{json, Value};

/// An error of the engine's public API. Store and kernel failures keep their codes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HostError {
    /// The store refused or could not perform an operation.
    #[error("store: {0}")]
    Store(#[from] StoreFailure),
    /// The kernel rejected a transition.
    #[error("kernel: {0}")]
    Kernel(#[from] Failure),
    /// The request was rejected by the engine before it reached the store.
    #[error("{code}: {message}")]
    Invalid { code: String, message: String },
    /// `output.invalid`: data that does not fit its outcome's schema; nothing was
    /// recorded.
    #[error("output.invalid: {0}")]
    OutputInvalid(Misfit),
    /// `input.invalid`: a run's input that does not fit the workflow's `inputSchema`;
    /// no run was created.
    #[error("input.invalid: {0}")]
    InputInvalid(InputMisfit),
    /// `signal.outcome-unrouted`: a signal for the node the run waits at, with an
    /// outcome the node does not route; nothing was recorded.
    #[error("signal.outcome-unrouted: {0}")]
    OutcomeUnrouted(Unrouted),
}

/// A signal refused at the node waiting for it: the outcome and the ones it accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unrouted {
    pub node_id: String,
    pub signal: String,
    pub outcome: String,
    pub outcomes: Vec<String>,
}

impl Unrouted {
    /// The details of a `signal.outcome-unrouted` error.
    pub fn details(&self) -> Value {
        json!({"nodeId": self.node_id, "signal": self.signal, "outcome": self.outcome,
               "outcomes": self.outcomes})
    }
}

/// The refusal, then the outcomes the node accepts.
impl fmt::Display for Unrouted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the run waits at {} for signal {}, which does not route outcome {:?}\n\
             accepted outcomes: {}",
            self.node_id,
            self.signal,
            self.outcome,
            self.outcomes.join(", ")
        )
    }
}

impl HostError {
    pub(crate) fn invalid(code: &str, message: impl Into<String>) -> Self {
        Self::Invalid {
            code: code.to_string(),
            message: message.into(),
        }
    }

    /// The machine-readable failure code: the store's, the kernel's, or the engine's.
    pub fn code(&self) -> &str {
        match self {
            Self::Store(f) => &f.code,
            Self::Kernel(f) => &f.code,
            Self::Invalid { code, .. } => code,
            Self::OutputInvalid(_) => runspore_schema::OUTPUT_INVALID,
            Self::InputInvalid(_) => runspore_schema::INPUT_INVALID,
            Self::OutcomeUnrouted(_) => code::SIGNAL_OUTCOME_UNROUTED,
        }
    }
}
